//! The link between the master and its workers in the Workers plugin
//! (ADR-0016, ADR-0031; docs/features/workers/pairing-and-link.md): the
//! protobuf contracts and the documents that cross them, mutual TLS in which
//! each side accepts only the other's pinned public key, pairing with a code,
//! and the worker's side of the link.
//!
//! The worker's side serves the runtime's assigned tasks (ADR-0028). The
//! master's side, with its connections to the paired workers and the
//! executor that gives them tasks, is `mannie-workers`. The core never
//! depends on the link.

pub mod codec;
pub mod host;
pub mod keys;
pub mod levels;
pub mod pairing;
pub mod proto;
#[cfg(feature = "testing")]
pub mod testing;
pub mod tls;
pub mod worker;
