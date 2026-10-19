//! The Workers plugin's test tools: the worker's side, a worker served over
//! the link on loopback with relays that break the link ([`worker`]), and the
//! master's side, the delegation harness and a master with a hub on loopback
//! ([`master`]). Only tests and the smoke test's master reach this crate, so
//! it may use the plugin harness and the host kit (ADR-0035, decision 6).

pub mod master;
pub mod worker;
