# ADR-0016: Connect the master and workers over gRPC with mutually pinned TLS

|          |                                                                                                                                                                           |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Proposed                                                                                                                                                                  |
| Date     | 2026-10-13                                                                                                                                                                |
| Deciders | Owner                                                                                                                                                                     |
| Related  | [ADR-0015](0015-run-an-agent-runtime-on-every-machine.md), [ADR-0010](0010-keep-provider-credentials-on-the-master.md), [Orchestration](../architecture/orchestration.md) |

## Context

Verified 2026-10-13:

- The link carries several kinds of traffic at the same time: task submissions, event streams, approval requests, credential leases and updates. It needs streaming, deadlines and cancellation, contracts that tolerate one minor version of difference between the two sides, and mutual authentication without a certificate authority.
- tonic 0.14.6, with its `tls-ring` feature, and prost 0.14.4 are the established gRPC stack for Rust.
- A WebSocket protocol with serde would need multiplexing, flow control and versioning built by hand. QUIC runs over UDP, which firewalls often block, and still needs an RPC layer on top.

## Decision

1. The master connects to the worker, and the worker listens on a configurable TCP port.
2. The link is gRPC over HTTP/2 with tonic and prost. Contracts live in `proto/mannie/link/v1/` as a buf module, checked by `buf lint` and `buf breaking`. A pure-Rust protobuf compiler (protox) generates the code during the build, so no system `protoc` is needed, and the generated code is never committed.
3. Both sides authenticate with mutual TLS, using rustls with ring. Each side has a long-term key pair. Pairing pins the hash of the peer's public key (its SubjectPublicKeyInfo), so certificates can be renewed without pairing again. Custom rustls verifiers accept only the pinned key. There is no certificate authority and there are no bearer tokens.
4. To pair, the worker prints a one-time code that is valid for 10 minutes. The master proves that it knows the code with an HMAC challenge that binds both public keys. Attempts are rate-limited. A worker is bound to one master, and `mannie-worker pair --reset` starts over.
5. The master keeps one persistent connection to each worker, with HTTP/2 keepalive. The state of that connection is the worker's status.
6. Each protobuf package has a feature level, and the two sides agree on levels when they connect. A master supports workers that are one minor release behind. Fields and services are only added, and removing one takes a deprecation release.
7. The services are Session (hello, feature levels and host facts), Tasks (idempotent submit, cancel and continue), Events (subscribe from a cursor), Approvals (the worker asks and the master answers), Leases (deliver and revoke credentials) and Update. The Update service's contract never changes, so the master can always update an older worker.
8. Worker release artifacts are signed with Ed25519. A worker checks the signature against the embedded release key before it installs an update.

## Consequences

- Positive: a mature, multiplexed transport with contract checks.
- Negative: the protobuf toolchain becomes part of the build, and the custom certificate verifiers need careful review and tests.

## Alternatives considered

| Option                                                | Why not chosen                                                       |
| ----------------------------------------------------- | -------------------------------------------------------------------- |
| WebSocket with serde                                  | Multiplexing, flow control and contract versioning built by hand     |
| QUIC                                                  | UDP is often blocked, and an RPC layer is still needed               |
| SSH tunnels                                           | Depends on SSH setup on every host and still needs a protocol on top |
| A new HTTPS request per call, with server-sent events | No multiplexing, and the connection state has to be guessed          |

## Validation

V6.
