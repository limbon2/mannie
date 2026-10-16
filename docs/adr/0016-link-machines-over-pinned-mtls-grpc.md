# ADR-0016: Connect the master and workers over gRPC with mutually pinned TLS

|          |                                                                                                                                                                           |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted                                                                                                                                                                  |
| Date     | 2026-10-13                                                                                                                                                                |
| Deciders | Owner                                                                                                                                                                     |
| Related  | [ADR-0015](0015-run-an-agent-runtime-on-every-machine.md), [ADR-0010](0010-keep-provider-credentials-on-the-master.md), [Orchestration](../architecture/orchestration.md) |

## Context

Verified 2026-10-13:

- The link carries several kinds of traffic at the same time: task submissions, event streams, approval requests, credential leases and updates. It needs streaming, deadlines and cancellation, contracts that tolerate one minor version of difference between the two sides, and mutual authentication without a certificate authority.
- tonic 0.14.6, with its `tls-ring` feature, and prost 0.14.4 are the established gRPC stack for Rust.
- A WebSocket protocol with serde would need multiplexing, flow control and versioning built by hand. QUIC runs over UDP, which firewalls often block, and still needs an RPC layer on top.

## Decision

1. The master connects to the worker, and the worker listens on a configurable TCP port, 7445 by default.
2. The link is gRPC over HTTP/2 with tonic and prost. Contracts live in `proto/mannie/link/<service>/v1/`, one package `mannie.link.<service>.v1` for each service, as a buf module checked by `buf lint` and `buf breaking`. A pure-Rust protobuf compiler (protox) generates the code during the build, so no system `protoc` is needed, and the generated code is never committed. Documents of the runtime, such as a task's state or an update of a conversation, travel as their JSON in protobuf fields; fixture tests guard them, since `buf breaking` does not see inside them.
3. Both sides authenticate with mutual TLS 1.3, using rustls with ring and raw public keys (RFC 7250) in place of certificates. Each side has a long-term Ed25519 key pair. Pairing pins the SHA-256 hash of the peer's public key (its SubjectPublicKeyInfo). Custom rustls verifiers accept only the pinned key and check the handshake's signature with it. There is no certificate, no certificate authority and no bearer token.
4. To pair, the worker prints a one-time code that is valid for 10 minutes: 20 characters of Crockford base32, which is 100 bits, because whoever captures one exchange can guess the code offline. Both sides prove that they know the code with HMACs over both nonces and both public keys, which each side takes from the TLS session itself; the worker proves it first. A code allows 5 attempts and works once. A worker is bound to one master, and `mannie-worker pair --reset` starts over.
5. The master keeps one persistent connection to each worker, with HTTP/2 keepalive. The state of that connection is the worker's status.
6. Each protobuf package has a feature level, and the two sides agree on levels when they connect. A master supports workers on its own minor release and one minor release behind, and shows any other worker as one to update. Fields and services are only added, and removing one takes a deprecation release.
7. The services are Session (hello, feature levels, host facts and unpairing), Pairing, Tasks (idempotent submit and continue, look-up, cancel, and a stream of task states from a version), Events (subscribe to a task conversation from a cursor, and its other reads), Approvals (the worker's requests travel in the task states, and the master answers them) and Leases (deliver and revoke credentials). There is no Update service: the owner updates a worker on its host ([decision D20](../roadmap/milestones.md#5-decisions)).
8. Worker release artifacts are signed with Ed25519. `mannie-worker update` checks the signature against the release key built into the worker before it installs a build (D20).

## Consequences

- Positive: a mature, multiplexed transport with contract checks.
- Negative: the protobuf toolchain becomes part of the build, and the custom verifiers need careful review and tests.
- Refinement in M4: decisions 1 to 4 and 6 to 8 were made concrete while the link was built ([pairing and the link](../features/workers/pairing-and-link.md)). Raw public keys replaced certificates, since only the pinned key matters and no X.509 has to be built or parsed. The owner removed the pushed updates and with them the Update service ([decision D20](../roadmap/milestones.md#5-decisions)).

## Alternatives considered

| Option                                                | Why not chosen                                                       |
| ----------------------------------------------------- | -------------------------------------------------------------------- |
| WebSocket with serde                                  | Multiplexing, flow control and contract versioning built by hand     |
| QUIC                                                  | UDP is often blocked, and an RPC layer is still needed               |
| SSH tunnels                                           | Depends on SSH setup on every host and still needs a protocol on top |
| A new HTTPS request per call, with server-sent events | No multiplexing, and the connection state has to be guessed          |

## Validation

V6.
