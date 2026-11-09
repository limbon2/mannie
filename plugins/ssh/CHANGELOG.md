# Changelog of the SSH plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Changed

- Hosts whose only host key is RSA can be scanned, trusted and used, with `rsa-sha2-512` or `rsa-sha2-256` signatures and keys of at least 2048 bits; SHA-1 `ssh-rsa` and shorter keys are still refused ([ADR-0002](docs/adr/0002-accept-rsa-host-keys.md)).
