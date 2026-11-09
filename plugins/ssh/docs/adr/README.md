# The SSH plugin's decision records

|              |                                                                                                                                    |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Living index                                                                                                                       |
| Last updated | 2026-11-09                                                                                                                         |
| Related      | [SSH hosts](../README.md), [Mannie's ADR index](../../../../docs/adr/README.md), [Template](../../../../docs/adr/0000-template.md) |

The SSH plugin's own decisions, numbered on their own ([ADR-0035](../../../../docs/adr/0035-find-plugins-at-build-time-in-folders-of-their-own.md), decision 11). A decision that changes the SDK or the core lives in [Mannie's index](../../../../docs/adr/README.md).

| ADR                                                       | Decision                                              | Status                                                                         | Validation   |
| --------------------------------------------------------- | ----------------------------------------------------- | ------------------------------------------------------------------------------ | ------------ |
| [0001](0001-speak-ssh-with-russh-and-pinned-host-keys.md) | Speak SSH with russh on ring, and pin each host's key | Accepted (D35), superseded in part by [ADR-0002](0002-accept-rsa-host-keys.md) | M8           |
| [0002](0002-accept-rsa-host-keys.md)                      | Accept RSA host keys signed with SHA-2                | Accepted (D53)                                                                 | Plugin tests |
