# Changelog of the SSH plugin

Each change to the plugin adds one line under Unreleased, below its kind of change, such as `### Added`, `### Changed` or `### Fixed` of [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). `task release:prepare` moves the lines into the release's section of [Mannie's changelog](../../CHANGELOG.md), under the plugin's name, and leaves this section empty.

## Unreleased

### Added

- The SSH plugin lets agents run commands on computers without Mannie. The owner registers a host in the SSH settings with its address, port, user name and password, which Mannie keeps in the system's credential store, and trusts its key after comparing the fingerprint; Mannie sends the password only to a host that shows that key, and refuses hosts that offer only RSA keys.
- The tool `run_ssh_command` runs one command on a registered host for the conversation agent and the `general` child agent. Each command asks the owner like a local command, with the host, its address and its key, and grants and rules cover only that host; full access runs commands without asking.
- A command's output reads as `run_command`'s, with long streams archived for `read_tool_output` and the host's password replaced. The timeout and Stop ask the host to end the command and close its channel.
- The system prompt lists the registered hosts, and "@" in the message box points an agent at one.
