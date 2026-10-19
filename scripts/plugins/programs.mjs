// @ts-check
// The programs that the plugins' manifests declare (ADR-0035, decision 10),
// for the tasks, CI and the release, so that none of them names a plugin or
// a program:
//
//   node scripts/plugins/programs.mjs build [--release] [--e2e]
//       builds every program, or those that the end-to-end tests use
//   node scripts/plugins/programs.mjs check
//       checks that no crate of Tauri, GTK or WebKitGTK reaches a headless
//       program on any target (V5, ADR-0008), and that the release builds an
//       archive for every target
//   node scripts/plugins/programs.mjs matrix
//       writes the matrices of CI and the release as `name=<json>` lines,
//       for $GITHUB_OUTPUT
//   node scripts/plugins/programs.mjs freebsd
//       prints the script that the nightly FreeBSD virtual machine runs
//   node scripts/plugins/programs.mjs freebsd-release
//       prints the script that builds and packs the FreeBSD archives
//   node scripts/plugins/programs.mjs archives <folder> <version> [--signed]
//       prints the release's archives of every program, or of the signed ones
//   node scripts/plugins/programs.mjs check-signed <folder> <version> <public key>
//       runs each signed program's static Linux build: its self-check must
//       report the release key, and it must accept the signature of each of
//       its archives
//   node scripts/plugins/programs.mjs sboms <folder> <version>
//       writes each program's CycloneDX SBOM into the folder
//   node scripts/plugins/programs.mjs notices <folder> <version>
//       writes each program's third-party notices into the folder

import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, renameSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

import { keyFingerprint } from '../release.mjs';

import { cargoMetadata } from './lib/cargo.mjs';
import {
  archiveName,
  buildCommands,
  dependencyClosure,
  FORBIDDEN_CRATE,
  freebsdReleaseScript,
  freebsdScript,
  matrices,
  readPrograms,
  unknownTargets,
} from './lib/programs.mjs';

/** @typedef {import('./lib/programs.mjs').Program} Program */

const root = resolve(import.meta.dirname, '../..');
const [command, ...options] = process.argv.slice(2);

/**
 * Stops with problems.
 * @param {string[]} problems
 * @returns {never}
 */
function fail(problems) {
  for (const problem of problems) {
    console.error(`error: ${problem}`);
  }
  process.exit(1);
}

/**
 * Runs a command in the root and stops when it fails.
 * @param {string} program
 * @param {string[]} args
 */
function run(program, args) {
  console.log(`programs: ${program} ${args.join(' ')}`);
  const result = spawnSync(program, args, { cwd: root, stdio: 'inherit' });
  if (result.status !== 0) {
    fail([`${program} ${args.join(' ')} failed`]);
  }
}

/**
 * The manifest of a program's package, from its workspace's metadata.
 * @param {Program} program
 */
function packageManifest(program) {
  const metadata = cargoMetadata(root, `${program.folder}/Cargo.toml`, { locked: true, noDeps: true, offline: false });
  if (!metadata.ok) {
    fail([`cargo metadata failed for ${program.folder}:\n${metadata.stderr}`]);
  }
  const found = metadata.metadata.packages.find((entry) => entry.name === program.package);
  if (found === undefined) {
    fail([`${program.folder}/plugin.toml names the program ${program.package}, which its workspace does not have`]);
  }
  return found.manifest_path;
}

/**
 * The arguments that a subcommand needs, or a usage error.
 * @param {number} count
 * @param {string} usage
 */
function argumentsOf(count, usage) {
  const given = options.filter((option) => !option.startsWith('--'));
  if (given.length < count) {
    console.error(`usage: node scripts/plugins/programs.mjs ${usage}`);
    process.exit(2);
  }
  return given;
}

const read = readPrograms(root);
if (read.problems.length > 0) {
  fail(read.problems);
}
const { programs } = read;

switch (command) {
  case 'build': {
    for (const args of buildCommands(programs, {
      e2e: options.includes('--e2e'),
      release: options.includes('--release'),
    })) {
      run('cargo', args);
    }
    break;
  }
  case 'check': {
    const problems = unknownTargets(programs);
    for (const program of programs.filter((candidate) => candidate.headless)) {
      const metadata = cargoMetadata(root, `${program.folder}/Cargo.toml`, { locked: true, offline: false });
      if (!metadata.ok) {
        problems.push(`cargo metadata failed for ${program.folder}:\n${metadata.stderr}`);
        continue;
      }
      const closure = dependencyClosure(metadata.metadata, program.package);
      const forbidden = [...closure].filter((name) => FORBIDDEN_CRATE.test(name)).sort();
      problems.push(
        ...forbidden.map(
          (name) =>
            `${program.package} depends on ${name}; a headless program must not depend on the desktop stack (ADR-0008, V5)`,
        ),
      );
      if (forbidden.length === 0) {
        console.log(
          `programs: none of the ${String(closure.size)} dependencies of ${program.package} belongs to Tauri, GTK or WebKitGTK`,
        );
      }
    }
    if (problems.length > 0) {
      fail(problems);
    }
    break;
  }
  case 'matrix': {
    for (const [name, value] of Object.entries(matrices(programs))) {
      console.log(`${name}=${JSON.stringify(value)}`);
    }
    break;
  }
  case 'freebsd': {
    process.stdout.write(freebsdScript(programs));
    break;
  }
  case 'freebsd-release': {
    process.stdout.write(freebsdReleaseScript(programs));
    break;
  }
  case 'archives': {
    const [folder, version] = argumentsOf(2, 'archives <folder> <version> [--signed]');
    const signedOnly = options.includes('--signed');
    for (const program of programs.filter((candidate) => candidate.signed || !signedOnly)) {
      for (const target of program.targets) {
        console.log(`${folder}/${archiveName(program.package, version, target)}`);
      }
    }
    break;
  }
  case 'check-signed': {
    const [folder, version, publicKey] = argumentsOf(3, 'check-signed <folder> <version> <public key>');
    const expected = keyFingerprint(readFileSync(publicKey, 'utf8'));
    for (const program of programs.filter((candidate) => candidate.signed)) {
      const linux = join(folder, archiveName(program.package, version, 'x86_64-unknown-linux-musl'));
      if (!existsSync(linux)) {
        fail([`the release checks the signed program ${program.package} with its static Linux build, ${linux}`]);
      }
      const unpacked = mkdtempSync(join(tmpdir(), `${program.package}-`));
      run('tar', ['-xzf', linux, '-C', unpacked]);
      const binary = join(unpacked, program.package);
      const check = spawnSync(binary, program.selfCheck, { cwd: root, encoding: 'utf8' });
      /** @type {{ releaseKeySha256?: string }} */
      const report = check.status === 0 ? JSON.parse(check.stdout) : {};
      if (report.releaseKeySha256 !== expected) {
        fail([
          `${program.package} checks updates with the key ${String(report.releaseKeySha256)}, not the release key ${expected}`,
        ]);
      }
      for (const target of program.targets) {
        run(binary, ['update', '--check', join(folder, archiveName(program.package, version, target))]);
      }
      rmSync(unpacked, { force: true, recursive: true });
    }
    break;
  }
  case 'sboms': {
    const [folder, version] = argumentsOf(2, 'sboms <folder> <version>');
    for (const program of programs) {
      const manifest = packageManifest(program);
      run('cargo', [
        'cyclonedx',
        '--manifest-path',
        manifest,
        '--format',
        'json',
        '--spec-version',
        '1.5',
        '--target',
        'all',
      ]);
      renameSync(
        join(dirname(manifest), `${program.package}.cdx.json`),
        join(folder, `${program.package}-${version}.cdx.json`),
      );
    }
    break;
  }
  case 'notices': {
    const [folder, version] = argumentsOf(2, 'notices <folder> <version>');
    for (const program of programs) {
      const output = join(folder, `THIRD-PARTY-NOTICES-${version}-${program.package}.md`);
      run('cargo', [
        'about',
        'generate',
        '--locked',
        '--config',
        'about.toml',
        '--manifest-path',
        packageManifest(program),
        '-o',
        output,
        'about.hbs',
      ]);
    }
    break;
  }
  default: {
    console.error(
      'usage: node scripts/plugins/programs.mjs (build [--release] [--e2e] | check | matrix | freebsd | freebsd-release | archives | check-signed | sboms | notices)',
    );
    process.exit(2);
  }
}
