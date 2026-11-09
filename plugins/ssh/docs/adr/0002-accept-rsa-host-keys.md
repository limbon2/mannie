# ADR-0002: Accept RSA host keys signed with SHA-2

|          |                                                                                                                                                                                                                                                                            |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D53)                                                                                                                                                                                                                                                             |
| Date     | 2026-11-09                                                                                                                                                                                                                                                                 |
| Deciders | Owner                                                                                                                                                                                                                                                                      |
| Related  | [ADR-0001](0001-speak-ssh-with-russh-and-pinned-host-keys.md), which this ADR supersedes in part, [SSH hosts](../README.md), [ADR-0017](../../../../docs/adr/0017-license-and-dependency-policy.md), [Technology stack](../../../../docs/architecture/technology-stack.md) |

## Context

- [ADR-0001](0001-speak-ssh-with-russh-and-pinned-host-keys.md) builds russh without its `rsa` feature, so a host that offers only RSA host keys is refused with `host_key_unsupported`. The owner has such hosts, and some devices (network appliances, older systems) cannot get another key type.
- RUSTSEC-2023-0071, the Marvin attack, is a timing side channel in the `rsa` crate's operations with a private key: decryption and signing. A stable fixed release still does not exist (verified 2026-11-09). russh 0.64.1's `rsa` feature pulls `rsa = "=0.10.0-rc.18"`, `pkcs1` and the `rsa` feature of `ssh-key 0.7.0-rc.11`.
- A client that checks a host key only verifies the host's signature over the exchange hash with the host's public key. That is a public-key operation and does not run the code that the advisory is about. The plugin has no RSA private key: it logs in with passwords only, and generates no client keys.
- `ssh-key` 0.7.0-rc.11 accepts an RSA public key of any size (verified 2026-11-09), so a minimum size has to be enforced by the plugin.
- The SHA-1 signature `ssh-rsa` is disabled by default since OpenSSH 8.8. OpenSSH servers with an RSA host key also sign with `rsa-sha2-256` and `rsa-sha2-512` (RFC 8332) since OpenSSH 7.2.

## Decision

1. The plugin builds russh 0.64.1 with `default-features = false, features = ["ring", "rsa"]`: still no aws-lc-rs and no compression. This supersedes the "no RSA" part of decision 1 of ADR-0001. The rest of ADR-0001 stays.
2. The client offers the host key algorithms Ed25519, ECDSA on P-256, P-384 and P-521, then `rsa-sha2-512` and `rsa-sha2-256`, in that order, so a host that has both keeps using its Ed25519 or ECDSA key. It never offers `ssh-rsa` with SHA-1.
3. An RSA host key under 2048 bits is refused with `host_key_unsupported`, on a scan and on a connection, before any authentication.
4. The plugin uses the `rsa` crate only through russh and `ssh-key` to verify host signatures with public keys. Production code never generates, loads or uses an RSA private key. The tests' SSH server generates one, which never leaves the test.
5. `deny.toml` ignores RUSTSEC-2023-0071 with this reason and is checked again when a release of `rsa` fixes the advisory, or when the plugin starts to use RSA private keys, such as for client keys, whichever comes first.

## Consequences

- Positive: the owner can register hosts whose only key is RSA, without changing them.
- Negative: the graph carries a crate with an open advisory under an ignore, and two pre-release crates (`rsa` 0.10.0-rc.18 and `pkcs1`), which russh pins. The ignore covers the whole graph, so a later use of RSA private keys anywhere would not fail cargo-deny. The recheck condition in `deny.toml` and this ADR limit that.
- Risks: an attacker who can observe timing learns nothing from verification, because no secret is involved. A 2048-bit RSA key is weaker than Ed25519 over the long term, and the owner can still add an Ed25519 key on the host, which the client prefers.

## Alternatives considered

| Option                               | Why not chosen                                                                                          |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| Keep refusing RSA host keys          | The owner has hosts that offer only RSA keys                                                            |
| Verify RSA with ring and patch russh | russh verifies through `ssh-key` and has no hook for another verifier, so the plugin would carry a fork |
| Also accept `ssh-rsa` with SHA-1     | SHA-1 signatures are deprecated, and every server that supports RSA keys since OpenSSH 7.2 offers SHA-2 |
| Wait for a fixed `rsa` release       | No date is known, and the advisory does not apply to verification                                       |

## Validation

The plugin's tests against the SSH server in the test's process: an RSA-only host with a 2048-bit key is scanned, pinned and runs a command; a host with only `ssh-rsa`, and a host with a 1024-bit RSA key, are refused with `host_key_unsupported` before any authentication; a host with both an Ed25519 and an RSA key pins the Ed25519 key. Then the owner's check against their own RSA-only host.
