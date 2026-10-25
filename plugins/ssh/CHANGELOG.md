# Changelog of the SSH plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Security

- ctutils 0.4.5, which fixes the advisory RUSTSEC-2026-0385 in the constant-time masks and `u128` comparisons that russh's ciphers and elliptic curves use.
- crypto-bigint 0.7.6, which fixes the advisory RUSTSEC-2026-0387 in the constant-time subtraction, division and Montgomery multiplication that russh's P-256, P-384 and P-521 curves use.
- tokio 1.53.3, which fixes the advisory RUSTSEC-2026-0386 in the constructors of `AsyncFd` on Unix, which neither the plugin's crates nor their dependencies call.
