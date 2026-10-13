# ADR-0017: License Mannie under Apache-2.0 and admit dependencies by license policy

|          |                                                                                                                                |
| -------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Proposed                                                                                                                       |
| Date     | 2026-10-13                                                                                                                     |
| Deciders | Owner                                                                                                                          |
| Related  | [ADR-0018](0018-release-from-tagged-builds.md), [Engineering standards](../engineering/standards.md), [LICENSE](../../LICENSE) |

## Context

Verified 2026-10-13:

- The owner chose Apache-2.0 ([decision D8](../roadmap/milestones.md#5-decisions)).
- Tauri depends on `dirs`, whose `dirs-sys` dependency pulls in `option-ext` under MPL-2.0.
- On Linux, the desktop app links the distribution's WebKitGTK and GTK, which are licensed LGPL-2.1-or-later. An AppImage would bundle them, together with the obligations that come with distributing them.

## Decision

1. Mannie is licensed under Apache-2.0, and `LICENSE` holds the text. `Cargo.toml` and `package.json` files declare Apache-2.0, and source files carry no per-file license headers.
2. Dependencies may use permissive licenses (MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, 0BSD, BSL-1.0, CC0-1.0, Unlicense), or MPL-2.0 when the library is used unmodified.
3. LGPL system libraries (WebKitGTK, GTK, the AppIndicator library that the system tray uses, and their dependencies) are allowed only when they are linked dynamically from the distribution's packages. On Linux, Mannie ships deb and rpm packages and no AppImage.
4. GPL and AGPL code never enters Mannie's code or binaries.
5. cargo-deny checks Rust licenses against the allow list, and a pnpm license check covers the frontend. A failure blocks the merge, and each exception is listed in `deny.toml` with its reason.
6. Release builds generate third-party notices.
7. While the owner is the only contributor, contributions need no sign-off. This is reconsidered before the repository accepts outside contributions.

## Consequences

- Positive: clear terms for Mannie and for everything it ships.
- Negative: some libraries are ruled out, and Linux users install from deb or rpm packages only.

## Alternatives considered

| Option               | Why not chosen                                           |
| -------------------- | -------------------------------------------------------- |
| MIT                  | No explicit patent grant                                 |
| No dependency policy | License problems would surface at release time, or never |

## Validation

V9.
