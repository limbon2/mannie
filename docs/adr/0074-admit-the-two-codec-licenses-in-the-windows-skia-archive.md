# ADR-0074: Admit the two codec licenses in the Windows Skia archive

|          |                                                                                                                                            |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted (D69)                                                                                                                             |
| Date     | 2026-11-18                                                                                                                                 |
| Deciders | Owner                                                                                                                                      |
| Related  | [Dependency licenses](0017-license-and-dependency-policy.md), [Native graphics](0073-host-native-graphics-surfaces-for-desktop-plugins.md) |

## Context

Verified 2026-11-18. The Windows x86_64 OpenGL archive for Rust Skia 0.157.0 contains object files from libpng and the libjpeg API of libjpeg-turbo. Both codecs are part of the upstream binary distribution. Cargo's license tools see the Rust bindings, which declare MIT. The native codec licenses fall outside the list in ADR-0017. [Pinned Skia dependencies](https://github.com/rust-skia/skia/blob/m157.2/DEPS), [bindings manifest](https://github.com/rust-skia/rust-skia/blob/0.157.0/skia-bindings/Cargo.toml).

## Decision

1. Allow the PNG Reference Library License version 2 for libpng revision `3061454d980de7d53608f594194cfac722721d2a` and the Independent JPEG Group license for the libjpeg API in libjpeg-turbo revision `640f254ad0fa03f6b1f29f89b7dd9366f2f6e533`. This exception applies only to Mannie's Windows x86_64 desktop build with the OpenGL, JPEG and PDF archive of Rust Skia 0.157.0 (Skia m157.2). [libpng license](https://skia.googlesource.com/third_party/libpng/+/3061454d980de7d53608f594194cfac722721d2a/LICENSE), [libjpeg-turbo licenses](https://chromium.googlesource.com/chromium/deps/libjpeg_turbo/+/640f254ad0fa03f6b1f29f89b7dd9366f2f6e533/LICENSE.md).
2. Distribute the native components' license texts and the Independent JPEG Group acknowledgment with the Windows desktop release. The release uses the version of the bindings in the launcher's lock to select the [notices](../../release/skia-0.157.0-notices.txt).
3. Review the native libraries, licenses and notices again when the archive version, target or feature selection changes. Record this exception in [deny.toml](../../deny.toml). Other dependencies continue under ADR-0017.

## Consequences

Windows builds can use upstream's prebuilt graphics library. The installer puts the native notices in `licenses/windows-graphics.txt` beside the executable. The release also carries them alongside its Cargo and npm notices.

## Alternatives considered

| Option                                           | Reason declined                                                        |
| ------------------------------------------------ | ---------------------------------------------------------------------- |
| Build a custom Skia archive without those codecs | Requires maintaining and verifying a separate native build on Windows. |

## Validation

Inspect the object files in the Windows archive against Skia's pinned dependency list. The Windows install check requires the notices in the installation. The release selects the matching notices and includes them in its checksummed files. [Releasing](../engineering/releasing.md).
