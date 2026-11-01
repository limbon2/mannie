//! What the host tells a plugin of itself (ADR-0034, decision 5): how the
//! host was built, and the contexts that a plugin's default and start
//! receive, with the services that its manifest declares.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::contracts::Clock;
use crate::error::{GRANT_MISSING, SERVICE_UNAVAILABLE, StartFailure};
use crate::ids::{FeatureId, PluginId};
use crate::service::{BoundService, Service};
use crate::services::store::ModuleQueries;
use crate::version::SDK_MAJOR;

/// How the host was built. It replaces the desktop app's cargo feature
/// `validation` for plugins, so a plugin can shorten a delay in validation
/// builds, such as Workers' wait before it tries a worker again, without a
/// feature of its own.
///
/// The core adds profiles by addition, so a plugin matches the ones that it
/// treats apart and lets every other one behave as a release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HostProfile {
    /// A build that Mannie ships.
    Release,
    /// A development build, such as `task dev` runs.
    Development,
    /// A validation build, which the end-to-end tests drive.
    Validation,
}

/// The host that runs a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct HostInfo {
    /// Mannie's version, such as `0.3.0`.
    pub version: String,
    /// The major of the plugin SDK that the host runs.
    pub sdk_major: u16,
    /// How the host was built.
    pub profile: HostProfile,
}

impl HostInfo {
    /// A host of Mannie's `version`, built as `profile`, which runs this
    /// SDK's major.
    pub fn new(version: impl Into<String>, profile: HostProfile) -> Self {
        Self {
            version: version.into(),
            sdk_major: SDK_MAJOR,
            profile,
        }
    }
}

/// What a plugin's start receives: the plugin's ID, the host, the clock of
/// every timestamp, and the services that the host bound to the plugin from
/// its manifest.
pub struct StartContext {
    plugin: PluginId,
    host: HostInfo,
    clock: Arc<dyn Clock>,
    /// The services that the manifest declares, by name.
    declared: BTreeSet<String>,
    /// The services that the host bound, by name.
    bound: BTreeMap<&'static str, BoundService>,
}

impl StartContext {
    /// The context of a start of `plugin` on `host`, with the services that
    /// its manifest declares and those that the host bound.
    #[cfg(feature = "host")]
    pub fn new(
        plugin: PluginId,
        host: HostInfo,
        clock: Arc<dyn Clock>,
        declared: BTreeSet<String>,
        bound: BTreeMap<&'static str, BoundService>,
    ) -> Self {
        Self {
            plugin,
            host,
            clock,
            declared,
            bound,
        }
    }

    /// The plugin's ID.
    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    /// The host that runs the plugin.
    pub fn host(&self) -> &HostInfo {
        &self.host
    }

    /// The clock of every timestamp.
    pub fn clock(&self) -> Arc<dyn Clock> {
        Arc::clone(&self.clock)
    }

    /// The service `S`, bound to the plugin. A service that the manifest
    /// does not declare fails with `grant_missing`, and a declared one that
    /// the host could not bind with `service_unavailable`, so a plugin
    /// returns the failure from its start as it is.
    pub fn service<S: Service>(&self) -> Result<Arc<S::Api>, StartFailure> {
        bound_service::<S>(&self.declared, &self.bound)
    }
}

/// The service `S` among the services that a manifest declares and the host
/// bound: `grant_missing` for an undeclared one, `service_unavailable` for a
/// declared one without its binding.
fn bound_service<S: Service>(
    declared: &BTreeSet<String>,
    bound: &BTreeMap<&'static str, BoundService>,
) -> Result<Arc<S::Api>, StartFailure> {
    if !declared.contains(S::NAME) {
        return Err(StartFailure::new(GRANT_MISSING));
    }
    bound
        .get(S::NAME)
        .and_then(BoundService::get::<S>)
        .ok_or_else(|| StartFailure::new(SERVICE_UNAVAILABLE))
}

/// What the start of a plugin's optional feature receives (ADR-0055,
/// decision 5): the plugin's ID, the feature's ID, the host, the clock of
/// every timestamp, and the services that the feature declares in the
/// plugin's manifest, which the host bound for the feature alone. The
/// plugin's own services stay with the plugin's instance, which starts the
/// feature.
pub struct FeatureContext {
    plugin: PluginId,
    feature: FeatureId,
    host: HostInfo,
    clock: Arc<dyn Clock>,
    /// The services that the feature declares, by name.
    declared: BTreeSet<String>,
    /// The services that the host bound for the feature, by name.
    bound: BTreeMap<&'static str, BoundService>,
}

impl FeatureContext {
    /// The context of a start of the feature `feature` of `plugin` on
    /// `host`, with the services that the feature declares and those that
    /// the host bound.
    #[cfg(feature = "host")]
    pub fn new(
        plugin: PluginId,
        feature: FeatureId,
        host: HostInfo,
        clock: Arc<dyn Clock>,
        declared: BTreeSet<String>,
        bound: BTreeMap<&'static str, BoundService>,
    ) -> Self {
        Self {
            plugin,
            feature,
            host,
            clock,
            declared,
            bound,
        }
    }

    /// The plugin's ID.
    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    /// The feature's ID.
    pub fn feature(&self) -> &FeatureId {
        &self.feature
    }

    /// The host that runs the plugin.
    pub fn host(&self) -> &HostInfo {
        &self.host
    }

    /// The clock of every timestamp.
    pub fn clock(&self) -> Arc<dyn Clock> {
        Arc::clone(&self.clock)
    }

    /// The service `S`, bound to the feature. A service that the feature
    /// does not declare fails with `grant_missing`, also one of its plugin,
    /// whose instance holds it already, and a declared one that the host
    /// could not bind with `service_unavailable`.
    pub fn service<S: Service>(&self) -> Result<Arc<S::Api>, StartFailure> {
        bound_service::<S>(&self.declared, &self.bound)
    }
}

// As for a plugin's start, only the names of the services show.
impl fmt::Debug for FeatureContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FeatureContext")
            .field("plugin", &self.plugin)
            .field("feature", &self.feature)
            .field("host", &self.host)
            .field("declared", &self.declared)
            .field("bound", &self.bound.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

// The clock and the services' APIs say nothing worth a log, and a service's
// API may reach the plugin's secrets, so only the names show.
impl fmt::Debug for StartContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StartContext")
            .field("plugin", &self.plugin)
            .field("host", &self.host)
            .field("declared", &self.declared)
            .field("bound", &self.bound.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// What a plugin's default receives: the host, and the queries of the
/// plugin's module of the store, for a plugin with a module.
#[derive(Debug, Clone)]
pub struct DefaultContext {
    host: HostInfo,
    store: Option<Arc<dyn ModuleQueries>>,
}

impl DefaultContext {
    /// The context of the default of a plugin on `host`, with the queries
    /// of its module, if it has one and the host could bind it.
    #[cfg(feature = "host")]
    pub fn new(host: HostInfo, store: Option<Arc<dyn ModuleQueries>>) -> Self {
        Self { host, store }
    }

    /// The host that runs the plugin.
    pub fn host(&self) -> &HostInfo {
        &self.host
    }

    /// The queries of the plugin's module of the store, or `None` for a
    /// plugin without a module, or one whose module the host could not
    /// bind.
    pub fn store(&self) -> Option<Arc<dyn ModuleQueries>> {
        self.store.clone()
    }
}

#[cfg(test)]
#[cfg(feature = "host")]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt;
    use std::sync::Arc;

    use super::{FeatureContext, HostInfo, HostProfile, StartContext};
    use crate::contracts::{Clock, UtcMillis};
    use crate::error::{GRANT_MISSING, SERVICE_UNAVAILABLE};
    use crate::ids::{FeatureId, PluginId};
    use crate::service::{BoundService, Service};
    use crate::version::SDK_MAJOR;

    trait Greeting: Send + Sync + fmt::Debug {
        fn greet(&self) -> String;
    }

    #[derive(Debug)]
    struct Fixed(&'static str);

    impl Greeting for Fixed {
        fn greet(&self) -> String {
            self.0.to_owned()
        }
    }

    enum Greeter {}

    impl Service for Greeter {
        const NAME: &'static str = "greeter";
        const MAJOR: u16 = 1;
        type Api = dyn Greeting;
    }

    enum Farewell {}

    impl Service for Farewell {
        const NAME: &'static str = "farewell";
        const MAJOR: u16 = 1;
        type Api = dyn Greeting;
    }

    enum Counter {}

    impl Service for Counter {
        const NAME: &'static str = "counter";
        const MAJOR: u16 = 1;
        type Api = dyn Greeting;
    }

    #[derive(Debug)]
    struct Stopped;

    impl Clock for Stopped {
        fn now(&self) -> UtcMillis {
            UtcMillis::from_millis(42)
        }
    }

    /// A start of `notes` that declares `greeter` and `farewell`, of which
    /// the host bound only `greeter`.
    fn context() -> StartContext {
        let api: Arc<dyn Greeting> = Arc::new(Fixed("hello"));
        StartContext::new(
            PluginId::parse("notes").unwrap(),
            HostInfo::new("0.3.0", HostProfile::Validation),
            Arc::new(Stopped),
            BTreeSet::from(["greeter".to_owned(), "farewell".to_owned()]),
            BTreeMap::from([("greeter", BoundService::new::<Greeter>(api))]),
        )
    }

    #[test]
    fn a_start_gets_the_declared_services_that_the_host_bound() {
        let context = context();

        let greeter = context.service::<Greeter>().unwrap();
        let unbound = context.service::<Farewell>().unwrap_err();
        let undeclared = context.service::<Counter>().unwrap_err();

        assert_eq!(greeter.greet(), "hello");
        assert_eq!(unbound.code(), SERVICE_UNAVAILABLE);
        assert_eq!(undeclared.code(), GRANT_MISSING);
    }

    #[test]
    fn a_start_knows_its_plugin_its_host_and_its_clock() {
        let context = context();

        assert_eq!(context.plugin().as_str(), "notes");
        assert_eq!(
            context.host(),
            &HostInfo {
                version: "0.3.0".to_owned(),
                sdk_major: SDK_MAJOR,
                profile: HostProfile::Validation,
            }
        );
        assert_eq!(context.clock().now(), UtcMillis::from_millis(42));
    }

    #[test]
    fn a_start_context_shows_the_names_of_its_services_and_not_their_apis() {
        let debug = format!("{:?}", context());

        assert!(debug.contains(r#"bound: ["greeter"]"#), "{debug}");
        assert!(!debug.contains("hello"), "{debug}");
    }

    /// A start of the feature `history` of `notes`, which declares only
    /// `farewell`, bound by the host.
    fn feature_context() -> FeatureContext {
        let api: Arc<dyn Greeting> = Arc::new(Fixed("goodbye"));
        FeatureContext::new(
            PluginId::parse("notes").unwrap(),
            FeatureId::parse("history").unwrap(),
            HostInfo::new("0.3.0", HostProfile::Validation),
            Arc::new(Stopped),
            BTreeSet::from(["farewell".to_owned()]),
            BTreeMap::from([("farewell", BoundService::new::<Farewell>(api))]),
        )
    }

    #[test]
    fn a_feature_gets_only_the_services_that_it_declares() {
        let context = feature_context();

        let farewell = context.service::<Farewell>().unwrap();
        let plugins = context.service::<Greeter>().unwrap_err();

        assert_eq!(farewell.greet(), "goodbye");
        assert_eq!(plugins.code(), GRANT_MISSING);
        assert_eq!(context.plugin().as_str(), "notes");
        assert_eq!(context.feature().as_str(), "history");
        assert_eq!(context.clock().now(), UtcMillis::from_millis(42));
    }

    #[test]
    fn a_feature_context_shows_the_names_of_its_services_and_not_their_apis() {
        let debug = format!("{:?}", feature_context());

        assert!(debug.contains(r#"bound: ["farewell"]"#), "{debug}");
        assert!(debug.contains("history"), "{debug}");
        assert!(!debug.contains("goodbye"), "{debug}");
    }
}
