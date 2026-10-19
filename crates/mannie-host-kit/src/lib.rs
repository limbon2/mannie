//! The host kit (ADR-0034, decision 12): what a program that a plugin ships
//! uses of the core, as the SDK is what a plugin's library crates use. Such a
//! program, such as the Workers plugin's worker, is a composition root of its
//! own: it opens a store, sets up logging, the provider and the tools, and
//! runs a runtime. The kit re-exports one by one what it needs, so a change
//! inside the core changes the kit, and the program keeps its paths.
//!
//! A plugin's programs and the library crates that its manifest lists under
//! them depend on it; nothing that a plugin's entry crate ships reaches it
//! (ADR-0035, decision 6).

pub mod logging;
pub mod provider;
pub mod runtime;
pub mod store;
pub mod tools;
pub mod types;
