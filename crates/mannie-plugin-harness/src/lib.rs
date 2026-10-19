//! The plugin harness (ADR-0034, decision 1): what a plugin's tests use of
//! the core, as the SDK is what its code uses. It hosts one plugin through
//! the real plugin host on the core's test runtime ([`host`]), checks that a
//! plugin keeps the SDK's rules ([`conformance`]), and re-exports one by one
//! the parts of the core that plugin tests drive and read: the runtime's test
//! tools ([`runtime`]), the provider's requests and fixtures ([`provider`]),
//! the store ([`store`]), the tools' rules ([`tools`]) and the contract types
//! that the SDK's contracts leave out ([`types`]).
//!
//! A plugin's crates take it as a dev-dependency, and a crate of a plugin
//! that only tests use may take it as a dependency; nothing that a plugin
//! ships reaches it (ADR-0035, decision 6). A change inside the core then
//! changes the harness, and the plugins' tests keep their paths.

pub mod conformance;
pub mod host;
pub mod provider;
pub mod runtime;
pub mod store;
pub mod tools;
pub mod types;
