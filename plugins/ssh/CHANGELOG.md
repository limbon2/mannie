# Changelog of the SSH plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- Terminals on the registered hosts: the plugin offers its hosts to the terminal plugin, whose terminals run the account's login shell on a pseudo terminal over the host's session, with the password replaced in their output ([ADR-0053](../../docs/adr/0053-let-plugins-offer-terminals.md)).

### Changed

- Hosts whose only host key is RSA can be scanned, trusted and used, with `rsa-sha2-512` or `rsa-sha2-256` signatures and keys of at least 2048 bits; SHA-1 `ssh-rsa` and shorter keys are still refused ([ADR-0002](docs/adr/0002-accept-rsa-host-keys.md)).

### Fixed

- A host that a command saved or trusted could keep showing the state that the command returned, when the list with its newer state arrived before the command's answer. The command's host now goes in only while no list arrived during the command.
