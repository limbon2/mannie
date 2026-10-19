# ADR-0001: Speak SSH with russh on ring, and pin each host's key

|          |                                                                                                                                                                                                                                                             |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D35)                                                                                                                                                                                                                                              |
| Date     | 2026-10-19                                                                                                                                                                                                                                                  |
| Deciders | Owner                                                                                                                                                                                                                                                       |
| Related  | [SSH hosts](../README.md), [ADR-0036](../../../../docs/adr/0036-let-plugins-run-commands-on-remote-hosts.md), [ADR-0017](../../../../docs/adr/0017-license-and-dependency-policy.md), [Technology stack](../../../../docs/architecture/technology-stack.md) |

## Context

- The SSH plugin logs in to hosts that the owner registers, with a user name and a password, and runs commands there ([ADR-0036](../../../../docs/adr/0036-let-plugins-run-commands-on-remote-hosts.md)). The password must reach only the host that the owner meant.
- Mannie's dependency policy bans aws-lc and fails on every advisory, and ring is its one crypto provider ([ADR-0017](../../../../docs/adr/0017-license-and-dependency-policy.md)). The `rsa` crate has RUSTSEC-2023-0071, a timing side channel, with no fixed release (verified 2026-10-19).
- russh 0.64.1 (Apache-2.0, released 2026-10-05) is an SSH implementation in Rust on tokio. Its default features are `flate2`, `aws-lc-rs` and `rsa`, and `ring` is its other crypto backend. `client::connect_stream` runs over a stream that the caller opens, `check_server_key` runs after the key exchange and before any authentication, and host certificates are off unless the caller asks for them. Its one advisory, RUSTSEC-2026-0154, was fixed in 0.60.3 (verified 2026-10-19).
- A spike on 2026-10-19 built russh 0.64.1 with `default-features = false, features = ["ring"]`: cargo-deny passed advisories, bans, licenses and sources on the four targets, and `cargo tree -i rsa` was empty. Its client logged in to its server with a password and with keyboard-interactive login, ran a command and read both streams and the exit status, sent a signal, and stopped before authentication when the pinned key differed.
- FreeBSD's `sshd_config` ships with `PasswordAuthentication no`, and takes passwords through keyboard-interactive login with PAM (verified 2026-10-19).

## Decision

1. The plugin speaks SSH through russh 0.64.1 with `default-features = false, features = ["ring"]`: no RSA, no aws-lc-rs and no compression. The client offers Ed25519 and ECDSA host keys, and russh's default ciphers, MACs and key exchanges, which leave out SHA-1, CBC and 3DES. A host that offers only RSA keys is refused with `host_key_unsupported`. Only this plugin may depend on russh, which its manifest's `[crates]` records.
2. The plugin opens the TCP connection itself, within 15 seconds for the connection and the handshake together, under its `[native] sockets` grant, and hands the stream to russh.
3. Each host has one trusted key. The owner trusts the key that a scan saw, by its SHA-256 fingerprint, after comparing it with the host's own; the plugin stores the key in OpenSSH's form and compares the whole public key after the key exchange. A host that shows another key gets nothing: the connection ends before authentication, and the plugin records the key that it saw for the owner to check. Host certificates are refused.
4. Login sends the password, and when the host takes no password directly but offers keyboard-interactive login, answers its questions only when there is exactly one hidden question, with the password. Any other login is `auth_method_unsupported`.
5. One session for each host is reused while its endpoint and trusted key stay the same, with a keepalive after 30 seconds of silence and at most 3 unanswered, and closes after 5 minutes without a call, when the host changes and when the plugin stops. A reused session that fails to open a channel is replaced once, since nothing has run at that point.

## Consequences

- Positive: the client is Rust in Mannie's process, so the password never reaches a child process or a terminal, and the plugin decides about host keys itself. The crypto stays on ring, and the graph has no crate with an open advisory.
- Negative: hosts with RSA keys only are refused, and the owner adds an Ed25519 or ECDSA key there. The plugin keeps its own known keys instead of the owner's `known_hosts`.
- Risks: russh is younger than OpenSSH, and a protocol detail of a server could fail the handshake, which `handshake_failed` reports. The owner's checks against OpenSSH on Linux, FreeBSD and Windows find such failures before v0.4.0.

## Alternatives considered

| Option                                        | Why not chosen                                                                                                  |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| The ssh2 crate                                | It binds libssh2 and OpenSSL, which the dependency policy and the worker's graph keep out                       |
| The openssh crate                             | It runs the system's `ssh` program, which takes no password from a program, and decides about host keys itself  |
| Running `ssh.exe` or `ssh` as a child process | The password would reach a child process or a pseudo-terminal                                                   |
| russh with its default features               | aws-lc-rs is banned, and the `rsa` crate has an advisory without a fixed release                                |
| Trust on first use                            | The first connection would send the password to whoever answers; the owner checks the fingerprint first instead |

## Validation

M8: the plugin's tests against an SSH server in the test's process, and the owner's checks against OpenSSH on Linux, FreeBSD and Windows.
