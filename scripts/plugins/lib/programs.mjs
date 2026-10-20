// @ts-check
// The programs that the plugins' manifests declare in `[[programs]]`
// (ADR-0035, decision 10), as the tasks, CI and the release read them, and
// the check that no crate of the desktop stack reaches a headless program
// on any target (V5, ADR-0008).

import { findPlugins, readRules } from './manifests.mjs';

/**
 * A program of a plugin.
 * @typedef {object} Program
 * @property {boolean} e2e whether the end-to-end tests use it
 * @property {string} folder its plugin's folder, relative to the root
 * @property {boolean} headless whether it runs without a window
 * @property {string} package its package, whose binary has the same name
 * @property {string} plugin its plugin's ID
 * @property {string[]} selfCheck the arguments of the release's check of the built program
 * @property {boolean} signed whether the release signs its archives
 * @property {{ helpers: string[], unix: string | null, windows: string | null }} smoke its smoke tests
 * @property {string[]} targets the targets that a release builds it for
 */

/**
 * @typedef {import('./cargo.mjs').Metadata} Metadata
 */

/** Crates of the desktop stack that must never reach a headless program. */
export const FORBIDDEN_CRATE = /^(?:tauri|wry|tao|muda|gtk|gdk|gio|glib|webkit2gtk|javascriptcore-rs|soup3)(?:[-_]|$)/u;

/**
 * The programs of the plugins that ship, in the order of the plugins'
 * starts. The fixture plugins ship nothing.
 * @param {string} root the repository's root
 * @returns {{ problems: string[], programs: Program[] }}
 */
export function readPrograms(root) {
  const found = findPlugins(root, readRules(root));
  if (found.problems.length > 0) {
    return { problems: found.problems, programs: [] };
  }
  const programs = found.plugins
    .filter((plugin) => !plugin.validation)
    .flatMap((plugin) =>
      (plugin.manifest.programs ?? []).map((program) => ({
        e2e: program.e2e ?? false,
        folder: plugin.folder,
        headless: program.headless ?? false,
        package: program.package,
        plugin: plugin.id,
        selfCheck: program['self-check'] ?? [],
        signed: program.signed ?? false,
        smoke: {
          helpers: program.smoke?.helpers ?? [],
          unix: program.smoke?.unix ?? null,
          windows: program.smoke?.windows ?? null,
        },
        targets: program.targets ?? [],
      })),
    );
  return { problems: [], programs };
}

/**
 * The names of the crates that a package depends on, directly or not,
 * through normal and build dependencies, on every target, in the resolve
 * graph of `cargo metadata`.
 * @param {Metadata} metadata
 * @param {string} root the package's name
 * @returns {Set<string>}
 */
export function dependencyClosure(metadata, root) {
  const names = new Map(metadata.packages.map((cratePackage) => [cratePackage.id, cratePackage.name]));
  const nodes = new Map((metadata.resolve?.nodes ?? []).map((node) => [node.id, node]));
  const rootPackage = metadata.packages.find((cratePackage) => cratePackage.name === root);
  if (rootPackage === undefined) {
    throw new Error(`cargo metadata has no package ${root}`);
  }
  /** @type {Set<string>} */
  const seen = new Set();
  const queue = [rootPackage.id];
  while (queue.length > 0) {
    const id = queue.pop() ?? '';
    for (const dependency of nodes.get(id)?.deps ?? []) {
      const shipped = dependency.dep_kinds.some((kind) => kind.kind !== 'dev');
      const isNew = !seen.has(dependency.pkg);
      if (shipped && isNew) {
        seen.add(dependency.pkg);
        queue.push(dependency.pkg);
      }
    }
  }
  return new Set([...seen].map((id) => names.get(id) ?? id));
}

/**
 * The cargo commands that build every program, or those that the
 * end-to-end tests use, each as its arguments.
 * @param {Program[]} programs
 * @param {{ e2e: boolean, release: boolean }} options
 * @returns {string[][]}
 */
export function buildCommands(programs, options) {
  return programs
    .filter((program) => program.e2e || !options.e2e)
    .map((program) => [
      'build',
      '--locked',
      ...(options.release ? ['--release'] : []),
      '--manifest-path',
      `${program.folder}/Cargo.toml`,
      '-p',
      program.package,
    ]);
}

/** The runners of CI's smoke tests, by the platform of a script. */
const SMOKE_RUNNERS = { unix: 'ubuntu-24.04', windows: 'windows-2025' };

/**
 * The platforms of the release's archives, by target (ADR-0008, ADR-0018):
 * an archive is `<package>-<version>-<platform>.<extension>`, which the
 * install checks read back by name alone.
 * @type {Record<string, { extension: string, platform: string }>}
 */
export const RELEASE_PLATFORMS = {
  'x86_64-pc-windows-msvc': { extension: 'zip', platform: 'windows-x86_64' },
  'x86_64-unknown-freebsd': { extension: 'tar.gz', platform: 'freebsd-x86_64' },
  'x86_64-unknown-linux-musl': { extension: 'tar.gz', platform: 'linux-x86_64' },
};

/**
 * The name of a program's release archive for a target.
 * @param {string} name the program's package
 * @param {string} version
 * @param {string} target
 */
export function archiveName(name, version, target) {
  const release = RELEASE_PLATFORMS[target];
  if (release === undefined) {
    throw new Error(`the release builds no archive for the target ${target}`);
  }
  return `${name}-${version}-${release.platform}.${release.extension}`;
}

/**
 * The targets of the programs that the release does not build archives for.
 * @param {Program[]} programs
 * @returns {string[]} one problem for each
 */
export function unknownTargets(programs) {
  return programs.flatMap((program) =>
    program.targets
      .filter((target) => !Object.hasOwn(RELEASE_PLATFORMS, target))
      .map(
        (target) =>
          `${program.folder}/plugin.toml: the release builds no archive for the target ${target} of ${program.package}`,
      ),
  );
}

/**
 * The matrices of CI and the release (ADR-0035, decision 10): every program;
 * one smoke test for each program and runner that has a script; one build for
 * each program and target that is not Windows, which CI makes on Linux with
 * cargo or, for FreeBSD, with cross; and the programs that the release builds
 * on Windows and as static Linux binaries.
 * @param {Program[]} programs
 */
export function matrices(programs) {
  const smoke = programs.flatMap((program) =>
    /** @type {const} */ (['unix', 'windows'])
      .filter((platform) => program.smoke[platform] !== null)
      .map((platform) => ({
        folder: program.folder,
        helpers: program.smoke.helpers.join(' '),
        os: SMOKE_RUNNERS[platform],
        package: program.package,
        script: `${program.folder}/${String(program.smoke[platform])}`,
      })),
  );
  const builds = programs.flatMap((program) =>
    program.targets
      .filter((target) => !target.includes('windows'))
      .map((target) => ({
        folder: program.folder,
        package: program.package,
        target,
        tool: target.includes('freebsd') ? 'cross' : 'cargo',
      })),
  );
  const windows = programs
    .filter((program) => program.targets.includes('x86_64-pc-windows-msvc'))
    .map((program) => ({ folder: program.folder, package: program.package }));
  const musl = builds.filter((build) => build.target.endsWith('-musl'));
  return { builds, musl, programs, smoke, windows };
}

/**
 * The shell script that the FreeBSD virtual machine of the nightly workflow
 * runs for the programs with a FreeBSD target: each plugin workspace's tests,
 * the program in the debug profile, its smoke helpers and its Unix smoke
 * test. It needs only cargo, so the machine needs no Node and no generated
 * file (ADR-0035, decision 10).
 * @param {Program[]} programs
 */
export function freebsdScript(programs) {
  const onFreebsd = programs.filter((program) => program.targets.some((target) => target.includes('freebsd')));
  const lines = ['set -eu'];
  for (const folder of new Set(onFreebsd.map((program) => program.folder))) {
    lines.push(`cargo test --locked --manifest-path ${folder}/Cargo.toml --workspace --all-features`);
  }
  for (const program of onFreebsd) {
    lines.push(`cargo build --locked --manifest-path ${program.folder}/Cargo.toml -p ${program.package}`);
    if (program.smoke.helpers.length > 0) {
      lines.push(
        `cargo build --locked --manifest-path ${program.folder}/Cargo.toml ${program.smoke.helpers.join(' ')}`,
      );
    }
    if (program.smoke.unix !== null) {
      lines.push(`sh ${program.folder}/${program.smoke.unix} target/debug`);
    }
  }
  return `${lines.join('\n')}\n`;
}

/**
 * The shell script that the release's FreeBSD virtual machine runs: each
 * program with a FreeBSD target in the release profile, packed with
 * `LICENSE` as its archive in `dist/`. `VERSION` holds the release's version.
 * @param {Program[]} programs
 */
export function freebsdReleaseScript(programs) {
  const lines = ['set -eu', 'mkdir -p dist'];
  for (const program of programs.filter((candidate) => candidate.targets.includes('x86_64-unknown-freebsd'))) {
    const archive = archiveName(program.package, '$VERSION', 'x86_64-unknown-freebsd');
    lines.push(
      `cargo build --locked --release --manifest-path ${program.folder}/Cargo.toml -p ${program.package}`,
      `mkdir -p staging/${program.package}`,
      `cp target/release/${program.package} LICENSE staging/${program.package}/`,
      `tar -czf "dist/${archive}" -C staging/${program.package} .`,
    );
  }
  return `${lines.join('\n')}\n`;
}
