//! What only the host uses: the providers that bind services to a plugin
//! (ADR-0034, decision 5), the consumers of the extension points, and the
//! coordinator that hands a plugin's contributions to them all or none
//! (decision 6).
//!
//! The crate that provides a service lists its providers, such as the
//! runtime's executor host and credentials, and the composition root hands
//! every list to the host once. At each start, the host asks the provider of
//! each service that the plugin's manifest declares to bind it to the
//! plugin, and grants nothing beyond the manifest.
//!
//! The host, and the core's test helpers that stand in for it, run one
//! coordinator of contributions, so the rules of IDs and the order of the
//! checks are written once. The coordinator applies the rules that need only
//! the plugin's own contributions: `Owned`, `PluginKind`, and two entries of
//! one point with one ID. A consumer applies what needs the core: a `Global`
//! ID that the core or another plugin has, and the checks of its point, such
//! as the name of a tool.
//!
//! A plugin's optional feature contributes as a [`Contributor`] of its own
//! (ADR-0055, decision 6): its contributions are added and dropped apart from
//! its plugin's, and its entries follow the rules of IDs of its plugin.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::BoxFuture;
use crate::context::HostProfile;
use crate::contracts::WorkItem;
use crate::contributions::{Contributions, IdRule, PointInfo};
use crate::ids::{Contributor, FeatureId, PluginId, owned_by};
use crate::manifest::Manifest;
use crate::service::BoundService;

/// A part of the core that provides one service, such as the runtime, which
/// provides the executor host. It binds the service to each plugin that
/// declares it, so the API that the plugin receives answers only for that
/// plugin: its executor kind, its secret entries or its store module.
pub trait ServiceProvider: Send + Sync + fmt::Debug + 'static {
    /// The name of the service that the provider binds, as the service's
    /// [`Service::NAME`](crate::service::Service::NAME).
    fn name(&self) -> &'static str;

    /// The major version of the service that the provider binds.
    fn major(&self) -> u16;

    /// Binds the service to one plugin. The plugin's ID, its manifest and
    /// the host's profile come from the binding. A provider that cannot bind
    /// the service to this plugin refuses, and the plugin fails to start
    /// with `service_unavailable`.
    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal>;

    /// Releases what the service holds for a contributor, once the plugin or
    /// the feature stopped or its start failed, such as the local servers
    /// that it started, which end with it (ADR-0043, decision 5). The host
    /// calls it for every provider: for a feature's services as the feature
    /// stops, and for the plugin's own as the plugin stops. A provider that
    /// holds nothing for a contributor keeps this default, which does
    /// nothing.
    fn release(&self, _contributor: &Contributor) -> BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }
}

/// The plugin that a provider binds a service to, and the feature for a
/// service that only the feature declares (ADR-0055, decision 5).
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct PluginBinding<'a> {
    /// The plugin's ID.
    pub id: &'a PluginId,
    /// The plugin's manifest, as the host checked it.
    pub manifest: &'a Manifest,
    /// How the host was built.
    pub profile: HostProfile,
    /// The feature that declares the service, or `None` for a service of the
    /// plugin itself.
    pub feature: Option<&'a FeatureId>,
}

impl<'a> PluginBinding<'a> {
    /// The binding of the plugin with this ID and manifest, on a host of
    /// this profile.
    pub fn new(id: &'a PluginId, manifest: &'a Manifest, profile: HostProfile) -> Self {
        Self {
            id,
            manifest,
            profile,
            feature: None,
        }
    }

    /// The same binding, for a service that the feature `feature` declares.
    #[must_use]
    pub fn for_feature(mut self, feature: &'a FeatureId) -> Self {
        self.feature = Some(feature);
        self
    }

    /// The contributor whose service this binds: the plugin, or its feature.
    pub fn contributor(&self) -> Contributor {
        match self.feature {
            None => Contributor::plugin(self.id.clone()),
            Some(feature) => Contributor::feature(self.id.clone(), feature.clone()),
        }
    }
}

/// Why a provider did not bind its service to a plugin. The plugin fails to
/// start with `service_unavailable`, and the log names the service.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the service {service} was not bound to the plugin: {detail}")]
#[non_exhaustive]
pub struct BindRefusal {
    /// The name of the service.
    pub service: &'static str,
    /// Why, for the log.
    pub detail: String,
}

impl BindRefusal {
    /// A refusal to bind `service`, with what the log says of it.
    pub fn new(service: &'static str, detail: impl Into<String>) -> Self {
        Self {
            service,
            detail: detail.into(),
        }
    }
}

/// The code of a contribution to a point that no consumer takes.
pub const NO_CONSUMER: &str = "no_consumer";
/// The code of an entry of an `Owned` point whose ID is neither the plugin's
/// ID nor starts with it and a dot.
pub const ID_NOT_OWNED: &str = "id_not_owned";
/// The code of an entry of a `PluginKind` point whose ID is not the plugin's.
pub const KIND_NOT_PLUGIN: &str = "kind_not_plugin";
/// The code of an entry whose ID the core, another plugin or another entry of
/// the plugin has.
pub const ID_TAKEN: &str = "id_taken";
/// The code of an entry whose name the point does not accept, such as a tool
/// whose name is not lower-case snake case.
pub const INVALID_NAME: &str = "invalid_name";
/// The code of a tool with an argument that the core keeps for itself.
pub const RESERVED_ARGUMENT: &str = "reserved_argument";
/// The code of a second entry of a `PluginKind` point, which takes one entry
/// from each plugin.
pub const TOO_MANY_ENTRIES: &str = "too_many_entries";
/// The code of a plugin, or a feature, whose contributions a consumer holds
/// already.
pub const PLUGIN_REGISTERED: &str = "plugin_registered";
/// The code of a feature's entry of a `PluginKind` point, which takes one
/// entry from each plugin and none from a feature (ADR-0055, decision 6).
pub const FEATURE_POINT: &str = "feature_point";

/// A part of the core that consumes the entries of some points, such as the
/// runtime, which consumes tools, agents, sections, executors, mention
/// providers and image sources.
///
/// The coordinator calls [`check`](Self::check) on every consumer before it
/// calls [`add`](Self::add) on any, and gives each only the entries of its
/// own points.
pub trait ContributionConsumer: Send + Sync + fmt::Debug + 'static {
    /// The points whose entries the consumer takes.
    fn points(&self) -> Vec<PointInfo>;

    /// Refuses a contributor's entries without taking any of them.
    fn check(
        &self,
        contributor: &Contributor,
        contributions: &Contributions,
    ) -> Result<(), ContributionRefusal>;

    /// Takes a contributor's entries, all or none. It checks them again,
    /// because another plugin may have taken an ID since
    /// [`check`](Self::check).
    fn add(
        &self,
        contributor: &Contributor,
        contributions: Contributions,
    ) -> Result<(), ContributionRefusal>;

    /// Drops a contributor's entries, and only those: a plugin's leaves its
    /// features' entries, which the host drops first. A contributor without
    /// entries changes nothing.
    fn remove(&self, contributor: &Contributor);

    /// Waits up to `grace` for the work that the contributor's entries
    /// started and that still runs, such as executions of its tools, then
    /// aborts the rest (ADR-0034, decision 8). The host calls it after
    /// [`remove`](Self::remove), so no new work starts meanwhile. A consumer
    /// whose entries start no such work has nothing to wait for.
    fn drain(&self, _contributor: &Contributor, _grace: Duration) -> BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }

    /// The work in progress that the contributor's entries handed the
    /// consumer and that turning it off ends, such as the background
    /// commands that its tools started (ADR-0041, decision 11). The host adds
    /// the items to the contributor's own, under codes that start with
    /// [`CORE_WORK_PREFIX`], which the core's UI words.
    fn work(&self, _contributor: &Contributor) -> Vec<WorkItem> {
        Vec::new()
    }
}

/// What the codes of the core's items of a plugin's work in progress start
/// with. A plugin's own codes never do.
pub const CORE_WORK_PREFIX: &str = "core.";

/// Why the core refused a plugin's contributions. It took none of them, and
/// the plugin fails to start with `contributions_refused`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the point {point} refused a contribution with the code {code}: {detail}")]
#[non_exhaustive]
pub struct ContributionRefusal {
    /// The name of the point whose entry was refused.
    pub point: &'static str,
    /// Why, as one of the codes of this module.
    pub code: &'static str,
    /// What was refused, for the log.
    pub detail: String,
}

impl ContributionRefusal {
    /// A refusal of an entry of `point`, with its code and what the log says
    /// of it.
    pub fn new(point: &'static str, code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            point,
            code,
            detail: detail.into(),
        }
    }
}

/// Refuses the entries that break their point's rule of IDs on their own:
/// an `Owned` ID that is not the plugin's, a `PluginKind` entry of a feature,
/// a `PluginKind` ID other than the plugin's ID or a second entry of such a
/// point, and two entries of one point with one ID. A feature's entries
/// follow the rules of its plugin's ID.
pub fn check_ids(
    contributor: &Contributor,
    contributions: &Contributions,
) -> Result<(), ContributionRefusal> {
    let plugin = contributor.plugin_id();
    let mut seen: Vec<(PointInfo, &str)> = Vec::new();
    for (point, id) in contributions.ids() {
        match point.ids {
            IdRule::PluginKind if contributor.feature_id().is_some() => {
                return Err(ContributionRefusal::new(
                    point.name,
                    FEATURE_POINT,
                    format!(
                        "the feature {contributor} contributes to a point that takes entries only from a plugin"
                    ),
                ));
            }
            IdRule::Owned if !owned_by(plugin.as_str(), id) => {
                return Err(ContributionRefusal::new(
                    point.name,
                    ID_NOT_OWNED,
                    format!(
                        "the entry {id:?} of the plugin {plugin} is neither the plugin's ID nor starts with it and a dot"
                    ),
                ));
            }
            IdRule::PluginKind if id != plugin.as_str() => {
                return Err(ContributionRefusal::new(
                    point.name,
                    KIND_NOT_PLUGIN,
                    format!("the entry {id:?} of the plugin {plugin} is not of the plugin's kind"),
                ));
            }
            IdRule::PluginKind if seen.iter().any(|(other, _id)| *other == point) => {
                return Err(ContributionRefusal::new(
                    point.name,
                    TOO_MANY_ENTRIES,
                    format!("the plugin {plugin} has more than one entry for the point"),
                ));
            }
            IdRule::Owned | IdRule::PluginKind | IdRule::Global => {}
        }
        if seen.contains(&(point, id)) {
            return Err(ContributionRefusal::new(
                point.name,
                ID_TAKEN,
                format!("the plugin {plugin} has two entries with the ID {id:?}"),
            ));
        }
        seen.push((point, id));
    }
    Ok(())
}

/// Hands a plugin's contributions to the consumers, all or none: it checks
/// the IDs, gives each point to the first consumer that takes it, refuses a
/// point that no consumer takes, lets every consumer check its part, and
/// only then lets each one add its part. A consumer whose add refuses, which
/// happens only when another plugin took an ID since the checks, makes the
/// consumers that added already remove the plugin again.
///
/// A consumer without entries of the contributor is not called.
pub fn add_contributions(
    consumers: &[Arc<dyn ContributionConsumer>],
    contributor: &Contributor,
    mut contributions: Contributions,
) -> Result<(), ContributionRefusal> {
    check_ids(contributor, &contributions)?;
    let mut parts: Vec<(&Arc<dyn ContributionConsumer>, Contributions)> = Vec::new();
    for consumer in consumers {
        let part = contributions.split_off(&consumer.points());
        if !part.is_empty() {
            parts.push((consumer, part));
        }
    }
    if let Some((point, _id)) = contributions.ids().next() {
        return Err(ContributionRefusal::new(
            point.name,
            NO_CONSUMER,
            format!(
                "no part of the core takes the point {} of major version {}",
                point.name, point.major
            ),
        ));
    }
    for (consumer, part) in &parts {
        consumer.check(contributor, part)?;
    }
    let mut added: Vec<&Arc<dyn ContributionConsumer>> = Vec::with_capacity(parts.len());
    for (consumer, part) in parts {
        if let Err(refusal) = consumer.add(contributor, part) {
            for earlier in added.iter().rev() {
                earlier.remove(contributor);
            }
            return Err(refusal);
        }
        added.push(consumer);
    }
    Ok(())
}

/// Drops a contributor's contributions from every consumer, the last first.
pub fn remove_contributions(
    consumers: &[Arc<dyn ContributionConsumer>],
    contributor: &Contributor,
) {
    for consumer in consumers.iter().rev() {
        consumer.remove(contributor);
    }
}

/// Waits for every consumer to drain the contributor's work at the same
/// time, each up to `grace` before it aborts the rest.
pub fn drain_contributions(
    consumers: &[Arc<dyn ContributionConsumer>],
    contributor: &Contributor,
    grace: Duration,
) -> BoxFuture<'static, ()> {
    let drains: Vec<_> = consumers
        .iter()
        .map(|consumer| consumer.drain(contributor, grace))
        .collect();
    Box::pin(async move {
        futures_util::future::join_all(drains).await;
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fmt;
    use std::sync::{Arc, Mutex, PoisonError};

    use super::{
        BindRefusal, ContributionConsumer, ContributionRefusal, FEATURE_POINT, ID_NOT_OWNED,
        ID_TAKEN, KIND_NOT_PLUGIN, NO_CONSUMER, PluginBinding, ServiceProvider, TOO_MANY_ENTRIES,
        add_contributions, check_ids, remove_contributions,
    };
    use crate::context::HostProfile;
    use crate::contributions::{Contributions, ExtensionPoint, IdRule, PointInfo};
    use crate::ids::{Contributor, FeatureId, PluginId};
    use crate::manifest::{CratesSection, Manifest, NativeSection, RustSection, SecretsSection};
    use crate::service::{BoundService, Service};

    /// A `Global` point whose entries are names.
    enum Names {}

    impl ExtensionPoint for Names {
        const NAME: &'static str = "names";
        const MAJOR: u16 = 1;
        const IDS: IdRule = IdRule::Global;
        type Entry = &'static str;

        fn id(entry: &Self::Entry) -> &str {
            entry
        }
    }

    /// An `Owned` point whose entries are names.
    enum Notes {}

    impl ExtensionPoint for Notes {
        const NAME: &'static str = "notes";
        const MAJOR: u16 = 1;
        const IDS: IdRule = IdRule::Owned;
        type Entry = &'static str;

        fn id(entry: &Self::Entry) -> &str {
            entry
        }
    }

    /// A `PluginKind` point whose entries are kinds.
    enum Kinds {}

    impl ExtensionPoint for Kinds {
        const NAME: &'static str = "kinds";
        const MAJOR: u16 = 1;
        const IDS: IdRule = IdRule::PluginKind;
        type Entry = &'static str;

        fn id(entry: &Self::Entry) -> &str {
            entry
        }
    }

    fn plugin() -> PluginId {
        PluginId::parse("fake").unwrap()
    }

    fn fake() -> Contributor {
        Contributor::plugin(plugin())
    }

    fn feature() -> Contributor {
        Contributor::feature(plugin(), FeatureId::parse("extra").unwrap())
    }

    /// The point and the code of a refusal.
    fn refused(result: Result<(), ContributionRefusal>) -> Option<(&'static str, &'static str)> {
        result.err().map(|refusal| (refusal.point, refusal.code))
    }

    #[test]
    fn owned_ids_are_the_plugins_id_or_start_with_it_and_a_dot() {
        let check =
            |id: &'static str| refused(check_ids(&fake(), &Contributions::new().add::<Notes>(id)));

        assert_eq!(check("fake"), None);
        assert_eq!(check("fake.notes"), None);
        for foreign in ["fakeish", "fake.", "other", "notes.fake", ""] {
            assert_eq!(check(foreign), Some(("notes", ID_NOT_OWNED)), "{foreign}");
        }
    }

    #[test]
    fn a_plugin_kind_is_the_plugins_id_and_a_plugin_has_one_entry_of_it() {
        let kinds = |ids: &[&'static str]| {
            let contributions = ids.iter().fold(Contributions::new(), |contributions, id| {
                contributions.add::<Kinds>(id)
            });
            refused(check_ids(&fake(), &contributions))
        };

        assert_eq!(kinds(&["fake"]), None);
        assert_eq!(kinds(&["other"]), Some(("kinds", KIND_NOT_PLUGIN)));
        assert_eq!(kinds(&["fake.box"]), Some(("kinds", KIND_NOT_PLUGIN)));
        assert_eq!(kinds(&["fake", "fake"]), Some(("kinds", TOO_MANY_ENTRIES)));
    }

    #[test]
    fn a_features_entries_follow_its_plugins_ids_and_never_take_a_plugin_kind() {
        let owned = Contributions::new()
            .add::<Notes>("fake.history")
            .add::<Names>("history");
        let foreign = Contributions::new().add::<Notes>("extra.history");
        let kind = Contributions::new().add::<Kinds>("fake");

        assert_eq!(refused(check_ids(&feature(), &owned)), None);
        assert_eq!(
            refused(check_ids(&feature(), &foreign)),
            Some(("notes", ID_NOT_OWNED))
        );
        assert_eq!(
            refused(check_ids(&feature(), &kind)),
            Some(("kinds", FEATURE_POINT))
        );
    }

    #[test]
    fn a_features_contributions_reach_the_consumers_as_its_own() {
        let log = Calls::default();
        let consumers: Vec<Arc<dyn ContributionConsumer>> = vec![Arc::new(Recording::new(
            "first",
            vec![PointInfo::of::<Names>()],
            &log,
        ))];

        let added = add_contributions(
            &consumers,
            &feature(),
            Contributions::new().add::<Names>("history"),
        );
        remove_contributions(&consumers, &feature());

        assert_eq!(added, Ok(()));
        assert_eq!(
            calls(&log),
            [
                r#"first check fake/extra ["history"]"#,
                r#"first add fake/extra ["history"]"#,
                "first remove fake/extra []",
            ]
        );
    }

    #[test]
    fn two_entries_of_one_point_with_one_id_are_refused_and_two_points_may_share_one() {
        let twice = Contributions::new()
            .add::<Names>("echo")
            .add::<Names>("echo");
        let apart = Contributions::new()
            .add::<Names>("fake")
            .add::<Notes>("fake")
            .add::<Kinds>("fake");

        assert_eq!(
            refused(check_ids(&fake(), &twice)),
            Some(("names", ID_TAKEN))
        );
        assert_eq!(refused(check_ids(&fake(), &apart)), None);
    }

    /// What a fake consumer was asked, in order.
    type Calls = Arc<Mutex<Vec<String>>>;

    /// A consumer of some points that records each call and can refuse its
    /// checks or its adds.
    #[derive(Debug)]
    struct Recording {
        name: &'static str,
        points: Vec<PointInfo>,
        refuse_checks: bool,
        refuse_adds: bool,
        calls: Calls,
    }

    impl Recording {
        fn new(name: &'static str, points: Vec<PointInfo>, calls: &Calls) -> Self {
            Self {
                name,
                points,
                refuse_checks: false,
                refuse_adds: false,
                calls: Arc::clone(calls),
            }
        }

        fn record(
            &self,
            call: &str,
            contributor: &Contributor,
            contributions: Option<&Contributions>,
        ) {
            let ids: Vec<&str> = contributions
                .map(|contributions| contributions.ids().map(|(_point, id)| id).collect())
                .unwrap_or_default();
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("{} {call} {contributor} {ids:?}", self.name));
        }
    }

    impl ContributionConsumer for Recording {
        fn points(&self) -> Vec<PointInfo> {
            self.points.clone()
        }

        fn check(
            &self,
            contributor: &Contributor,
            contributions: &Contributions,
        ) -> Result<(), ContributionRefusal> {
            self.record("check", contributor, Some(contributions));
            if self.refuse_checks {
                return Err(ContributionRefusal::new("names", ID_TAKEN, "taken"));
            }
            Ok(())
        }

        fn add(
            &self,
            contributor: &Contributor,
            contributions: Contributions,
        ) -> Result<(), ContributionRefusal> {
            self.record("add", contributor, Some(&contributions));
            if self.refuse_adds {
                return Err(ContributionRefusal::new("notes", ID_TAKEN, "taken since"));
            }
            Ok(())
        }

        fn remove(&self, contributor: &Contributor) {
            self.record("remove", contributor, None);
        }
    }

    fn calls(calls: &Calls) -> Vec<String> {
        calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn both() -> Contributions {
        Contributions::new()
            .add::<Notes>("fake.notes")
            .add::<Names>("echo")
    }

    #[test]
    fn every_consumer_checks_its_own_part_before_any_adds_it() {
        let log = Calls::default();
        let consumers: Vec<Arc<dyn ContributionConsumer>> = vec![
            Arc::new(Recording::new(
                "first",
                vec![PointInfo::of::<Names>()],
                &log,
            )),
            Arc::new(Recording::new(
                "second",
                vec![PointInfo::of::<Notes>()],
                &log,
            )),
            Arc::new(Recording::new("idle", vec![PointInfo::of::<Kinds>()], &log)),
        ];

        let added = add_contributions(&consumers, &fake(), both());
        remove_contributions(&consumers, &fake());

        assert_eq!(added, Ok(()));
        assert_eq!(
            calls(&log),
            [
                r#"first check fake ["echo"]"#,
                r#"second check fake ["fake.notes"]"#,
                r#"first add fake ["echo"]"#,
                r#"second add fake ["fake.notes"]"#,
                "idle remove fake []",
                "second remove fake []",
                "first remove fake []",
            ]
        );
    }

    #[test]
    fn a_refused_check_lets_no_consumer_add_anything() {
        let log = Calls::default();
        let refusing = Recording {
            refuse_checks: true,
            ..Recording::new("second", vec![PointInfo::of::<Notes>()], &log)
        };
        let consumers: Vec<Arc<dyn ContributionConsumer>> = vec![
            Arc::new(Recording::new(
                "first",
                vec![PointInfo::of::<Names>()],
                &log,
            )),
            Arc::new(refusing),
        ];

        let added = add_contributions(&consumers, &fake(), both());

        assert_eq!(refused(added), Some(("names", ID_TAKEN)));
        assert_eq!(
            calls(&log),
            [
                r#"first check fake ["echo"]"#,
                r#"second check fake ["fake.notes"]"#,
            ]
        );
    }

    #[test]
    fn a_refused_add_makes_the_consumers_that_added_already_remove_the_plugin() {
        let log = Calls::default();
        let refusing = Recording {
            refuse_adds: true,
            ..Recording::new("second", vec![PointInfo::of::<Notes>()], &log)
        };
        let consumers: Vec<Arc<dyn ContributionConsumer>> = vec![
            Arc::new(Recording::new(
                "first",
                vec![PointInfo::of::<Names>()],
                &log,
            )),
            Arc::new(refusing),
        ];

        let added = add_contributions(&consumers, &fake(), both());

        assert_eq!(refused(added), Some(("notes", ID_TAKEN)));
        assert_eq!(
            calls(&log),
            [
                r#"first check fake ["echo"]"#,
                r#"second check fake ["fake.notes"]"#,
                r#"first add fake ["echo"]"#,
                r#"second add fake ["fake.notes"]"#,
                "first remove fake []",
            ]
        );
    }

    #[test]
    fn a_point_that_no_consumer_takes_refuses_the_plugin_before_any_check() {
        let log = Calls::default();
        let consumers: Vec<Arc<dyn ContributionConsumer>> = vec![Arc::new(Recording::new(
            "first",
            vec![PointInfo::of::<Names>()],
            &log,
        ))];

        let added = add_contributions(&consumers, &fake(), both());
        let refusal = added.unwrap_err();

        assert_eq!((refusal.point, refusal.code), ("notes", NO_CONSUMER));
        assert!(refusal.detail.contains("notes"), "{}", refusal.detail);
        assert_eq!(calls(&log), Vec::<String>::new());
    }

    #[test]
    fn ids_are_checked_before_any_consumer_is_asked() {
        let log = Calls::default();
        let consumers: Vec<Arc<dyn ContributionConsumer>> = vec![Arc::new(Recording::new(
            "first",
            vec![PointInfo::of::<Notes>()],
            &log,
        ))];

        let added = add_contributions(
            &consumers,
            &fake(),
            Contributions::new().add::<Notes>("other.notes"),
        );

        assert_eq!(refused(added), Some(("notes", ID_NOT_OWNED)));
        assert_eq!(calls(&log), Vec::<String>::new());
    }

    #[test]
    fn a_refusal_says_which_point_refused_what() {
        let refusal =
            ContributionRefusal::new("tools", ID_TAKEN, "a tool named read_file is the core's");

        assert_eq!(
            refusal.to_string(),
            "the point tools refused a contribution with the code id_taken: a tool named read_file is the core's"
        );
    }

    /// What two test services share as their API.
    trait Greeting: Send + Sync + fmt::Debug {
        fn greet(&self) -> String;
    }

    /// A greeting that answers with a fixed text.
    #[derive(Debug)]
    struct Fixed(String);

    impl Greeting for Fixed {
        fn greet(&self) -> String {
            self.0.clone()
        }
    }

    enum Greeter {}

    impl Service for Greeter {
        const NAME: &'static str = "greeter";
        const MAJOR: u16 = 1;
        type Api = dyn Greeting;
    }

    /// A second service with the same API type as `Greeter`.
    enum Farewell {}

    impl Service for Farewell {
        const NAME: &'static str = "farewell";
        const MAJOR: u16 = 1;
        type Api = dyn Greeting;
    }

    /// A service with an API type of its own.
    enum Counter {}

    impl Service for Counter {
        const NAME: &'static str = "counter";
        const MAJOR: u16 = 1;
        type Api = Mutex<u64>;
    }

    fn manifest() -> Manifest {
        Manifest {
            id: plugin(),
            sdk: 1,
            order: 10,
            rust: RustSection {
                entry: "mannie-fake".to_owned(),
            },
            services: BTreeMap::from([("greeter".to_owned(), 1)]),
            secrets: SecretsSection::default(),
            crates: CratesSection::default(),
            programs: Vec::new(),
            processes: BTreeMap::new(),
            native: NativeSection::default(),
            features: BTreeMap::new(),
        }
    }

    /// Binds `Greeter` to plugins that declare it, greeting with the
    /// plugin's ID and the host's profile.
    #[derive(Debug)]
    struct GreeterProvider;

    impl ServiceProvider for GreeterProvider {
        fn name(&self) -> &'static str {
            Greeter::NAME
        }

        fn major(&self) -> u16 {
            Greeter::MAJOR
        }

        fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
            if !binding.manifest.declares(Greeter::NAME) {
                return Err(BindRefusal::new(
                    Greeter::NAME,
                    format!("the plugin {} does not declare it", binding.id),
                ));
            }
            let text = format!("hello {} on {:?}", binding.id, binding.profile);
            let api: Arc<dyn Greeting> = Arc::new(Fixed(text));
            Ok(BoundService::new::<Greeter>(api))
        }
    }

    #[test]
    fn a_bound_service_gives_its_api_to_its_own_service_alone() {
        let api: Arc<dyn Greeting> = Arc::new(Fixed("hello".to_owned()));
        let bound = BoundService::new::<Greeter>(api);

        let greeter = bound.get::<Greeter>().unwrap();

        assert_eq!(greeter.greet(), "hello");
        assert!(bound.get::<Farewell>().is_none());
        assert!(bound.get::<Counter>().is_none());
    }

    #[test]
    fn a_bound_service_shows_the_name_of_its_service_and_not_its_api() {
        let api: Arc<dyn Greeting> = Arc::new(Fixed("planted greeting".to_owned()));
        let bound = BoundService::new::<Greeter>(api);

        let debug = format!("{bound:?}");

        assert_eq!(debug, r#"BoundService { service: "greeter", .. }"#);
    }

    #[test]
    fn a_provider_binds_its_service_to_the_plugin_on_the_hosts_profile() {
        let providers: Vec<Arc<dyn ServiceProvider>> = vec![Arc::new(GreeterProvider)];
        let id = plugin();
        let declaring = manifest();
        let silent = Manifest {
            services: BTreeMap::new(),
            ..manifest()
        };

        let bound = providers[0]
            .bind(&PluginBinding::new(
                &id,
                &declaring,
                HostProfile::Validation,
            ))
            .unwrap();
        let refused = providers[0]
            .bind(&PluginBinding::new(&id, &silent, HostProfile::Release))
            .unwrap_err();

        assert_eq!((providers[0].name(), providers[0].major()), ("greeter", 1));
        assert_eq!(
            bound.get::<Greeter>().unwrap().greet(),
            "hello fake on Validation"
        );
        assert_eq!(
            refused,
            BindRefusal::new("greeter", "the plugin fake does not declare it")
        );
    }

    #[test]
    fn a_binding_names_its_contributor_the_plugin_or_its_feature() {
        let id = plugin();
        let extra = FeatureId::parse("extra").unwrap();
        let declaring = manifest();
        let binding = PluginBinding::new(&id, &declaring, HostProfile::Release);

        assert_eq!(binding.feature, None);
        assert_eq!(binding.contributor(), fake());
        assert_eq!(binding.for_feature(&extra).feature, Some(&extra));
        assert_eq!(binding.for_feature(&extra).contributor(), feature());
    }

    #[test]
    fn a_refusal_to_bind_names_the_service_and_why() {
        let refusal = BindRefusal::new("secrets", "the name link/key is not the plugin's");

        assert_eq!(
            refusal.to_string(),
            "the service secrets was not bound to the plugin: the name link/key is not the plugin's"
        );
    }
}
