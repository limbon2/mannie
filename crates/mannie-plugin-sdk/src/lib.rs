//! The plugin SDK (ADR-0034): the Rust surface that Mannie's plugins build on,
//! and their bridge to the core. A plugin's library crates reach the core
//! through this crate (decision 2), so a change inside the runtime or the
//! store does not reach them, and when a plugin needs something that the SDK
//! lacks, the SDK changes.
//!
//! The SDK follows protocol rules (decision 3), so that a later adapter could
//! carry its calls across a process or WebAssembly boundary: arguments and
//! results are owned values that serialize, methods return futures and
//! streams that own their data and are cancelled by dropping them, and the
//! host never hands a plugin a callback into host code. `tests/protocol.rs`
//! round-trips the SDK's values and lists the shapes that break these rules,
//! each with its reason.
//!
//! A plugin's manifest ([`manifest`]) names the SDK major that the plugin
//! builds on and the services that it uses ([`services`]). Its code
//! implements [`plugin::Plugin`], whose start receives the services through
//! its [`context`] and returns what the plugin contributes to the extension
//! points ([`points`]) and the handlers of its commands and channels
//! ([`ipc`]) while it is on, and its entry crate calls [`export_plugin!`].
//!
//! Each item that the SDK defines has one path, in its module, and each item
//! of `mannie-types` that it re-exports has one path, in [`contracts`].

#[cfg(feature = "bindings")]
pub mod bindings;
pub mod context;
pub mod contracts;
pub mod contributions;
pub mod error;
#[cfg(feature = "host")]
pub mod host;
pub mod ids;
pub mod ipc;
pub mod manifest;
pub mod plugin;
pub mod points;
pub mod service;
pub mod services;
#[cfg(feature = "testing")]
pub mod testing;
pub mod version;

// The SDK's methods return these types (decision 3), so a plugin implements
// the SDK's traits without a futures-util of its own.
pub use futures_util::future::BoxFuture;
pub use futures_util::stream::BoxStream;
