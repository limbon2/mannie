//! The host of Mannie's plugins (ADR-0030; ADR-0034, decisions 4 to 6 and
//! 8): it takes the list of plugins with their manifests, starts the plugins
//! that the owner turned on, binds each one the services that its manifest
//! declares, hands what it contributes to the core's consumers, dispatches
//! the calls of its commands and channels, stops it, and keeps the owner's
//! choices.
//!
//! A plugin implements the SDK's `Plugin` and depends on nothing here. The
//! desktop app's composition root lists the plugins and hands the host the
//! core's providers of services and consumers of extension points, so the
//! host knows no part of the core but the store, which keeps the owner's
//! choices and the plugins' modules.

mod dispatch;
mod host;

pub use dispatch::CallFailure;
pub use host::{HostError, PluginHost, PluginHostDeps, PluginList};
