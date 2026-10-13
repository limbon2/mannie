// @ts-check
// Checks that every crate of the Cargo workspace inherits the workspace lint
// policy with `[lints] workspace = true` (see Cargo.toml). A crate without it
// compiles under Rust's default lints, and Clippy would pass it silently.
// `task lint:rust` pipes `cargo metadata --format-version 1 --no-deps` into
// this script.

import { readFileSync } from 'node:fs';

/** @type {{ packages: { name: string, manifest_path: string }[] }} */
const metadata = JSON.parse(readFileSync(process.stdin.fd, 'utf8'));

// The three ways TOML can spell the setting.
const inheritsWorkspaceLints = [
  /^\[lints\][ \t]*\r?\n(?:[ \t]*(?:#.*)?\r?\n)*[ \t]*workspace[ \t]*=[ \t]*true\b/m,
  /^lints\.workspace[ \t]*=[ \t]*true\b/m,
  /^lints[ \t]*=[ \t]*\{[ \t]*workspace[ \t]*=[ \t]*true[ \t]*\}/m,
];

const crates = metadata.packages;
const missing = crates.filter((crate) => {
  const manifest = readFileSync(crate.manifest_path, 'utf8');
  return !inheritsWorkspaceLints.some((pattern) => pattern.test(manifest));
});

for (const crate of missing) {
  console.error(
    `error: ${crate.manifest_path}: add "[lints] workspace = true" so ${crate.name} follows the workspace lints`,
  );
}
if (missing.length > 0) {
  process.exitCode = 1;
} else {
  console.log(`check-workspace-lints: ${crates.length} crates inherit the workspace lints`);
}
