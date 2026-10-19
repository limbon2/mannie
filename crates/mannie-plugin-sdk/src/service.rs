//! Services (ADR-0034, decision 5): what the core gives a plugin that its
//! manifest declares, such as the rows of its executor's tasks.
//!
//! A service has a name, as `plugin.toml` names it under `[services]`, a
//! major version, a flag for services that reach sensitive data, and the API
//! that the plugin calls. The services of SDK 1 are in
//! [`services`](crate::services), each named by a marker type. The core
//! module that provides a service binds it to each plugin that declares it,
//! so the API that a plugin receives already answers only for that plugin,
//! and the host grants nothing beyond the manifest.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

/// A service of the SDK. The marker type names it, and its `Api` is what a
/// plugin calls.
pub trait Service: 'static {
    /// The service's name, as a manifest declares it under `[services]`:
    /// lower-case words joined by hyphens, such as `executor-host`, unique
    /// among the services.
    const NAME: &'static str;
    /// The service's major version, which a manifest names. An additive
    /// change keeps it (decision 14).
    const MAJOR: u16;
    /// Whether the service reaches sensitive data, such as provider
    /// credentials. The Plugins page marks such a service, and
    /// `task plugins:grants` lists it (decision 13).
    const SENSITIVE: bool = false;
    /// What a plugin calls, as the provider bound it to the plugin.
    type Api: ?Sized + Send + Sync + 'static;
}

/// What the host knows of a service without its types: its name, its major
/// version and whether it is sensitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServiceInfo {
    /// The service's name.
    pub name: &'static str,
    /// The service's major version.
    pub major: u16,
    /// Whether the service reaches sensitive data.
    pub sensitive: bool,
}

impl ServiceInfo {
    /// What a service declares of itself.
    pub const fn of<S: Service>() -> Self {
        Self {
            name: S::NAME,
            major: S::MAJOR,
            sensitive: S::SENSITIVE,
        }
    }
}

/// A service bound to one plugin: the API that the plugin receives, for the
/// service that the provider named. A provider makes it, and the plugin's
/// start reaches it through its context.
pub struct BoundService {
    /// The name of the service.
    service: &'static str,
    /// The service's API, an `Arc<S::Api>` of the service `S`.
    api: Box<dyn Any + Send + Sync>,
}

impl BoundService {
    /// The API of the service `S`, bound to one plugin.
    #[cfg(feature = "host")]
    pub fn new<S: Service>(api: Arc<S::Api>) -> Self {
        Self {
            service: S::NAME,
            api: Box::new(api),
        }
    }

    /// The API, when it is the service `S`'s. Two services may share an API
    /// type, so the name decides as well as the type.
    pub fn get<S: Service>(&self) -> Option<Arc<S::Api>> {
        if self.service != S::NAME {
            return None;
        }
        self.api.downcast_ref::<Arc<S::Api>>().map(Arc::clone)
    }
}

impl fmt::Debug for BoundService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundService")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}
