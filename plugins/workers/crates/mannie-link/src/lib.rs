//! The link between the master and its workers in the Workers plugin
//! (ADR-0016, ADR-0031; plugins/workers/docs/pairing-and-link.md): the
//! protobuf contracts and the documents that cross them, mutual TLS in which
//! each side accepts only the other's pinned public key, and pairing with a
//! code. It builds on the plugin SDK alone (ADR-0034, decision 12).
//!
//! The worker's side, which serves the runtime's assigned tasks (ADR-0028),
//! is `mannie-link-worker`, which only the worker uses. The master's side,
//! with its connections to the paired workers and the executor that gives
//! them tasks, is `mannie-workers`. The core never depends on the link.

pub mod codec;
pub mod host;
pub mod keys;
pub mod levels;
pub mod pairing;
pub mod proto;
pub mod tls;
