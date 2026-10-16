//! The link between the master and its workers (ADR-0016; docs/features/
//! workers/pairing-and-link.md): the protobuf contracts, mutual TLS in which
//! each side accepts only the other's pinned public key, pairing with a code,
//! and the adapters of both sides to the agent runtime.
//!
//! The master's side implements the runtime's `Delegation` port (ADR-0027);
//! the worker's side serves the runtime's assigned tasks (ADR-0028). The
//! runtime never depends on the link.

pub mod keys;
pub mod levels;
pub mod master;
pub mod pairing;
pub mod proto;
#[cfg(feature = "testing")]
pub mod testing;
pub mod tls;
pub mod worker;

mod codec;
