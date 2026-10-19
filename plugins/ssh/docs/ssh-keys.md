# SSH keys

|              |                                                                                                                                    |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                               |
| Milestone    | Unscheduled                                                                                                                        |
| Last updated | 2026-10-20                                                                                                                         |
| Part of      | [SSH hosts](README.md)                                                                                                             |
| Related      | [ADR-0036](../../../docs/adr/0036-let-plugins-run-commands-on-remote-hosts.md), [Security](../../../docs/architecture/security.md) |

## Intent

The owner deferred SSH keys from the first version ([D35](../../../docs/roadmap/milestones.md#5-decisions)). With a key, Mannie would log in to a host without sending it a password, so a compromised host learns nothing that opens another, and the host can limit what the key may do.

## Open questions

- Whether Mannie makes the key itself, keeping its private half in the credential store, or uses a key that the owner already has, such as through the system's SSH agent.
- Which key types it offers: Ed25519 alone, or ECDSA too. RSA stays out while the `rsa` crate has its advisory.
- How the owner puts the public key on a host: by copying it, or by Mannie installing it after one password login.
