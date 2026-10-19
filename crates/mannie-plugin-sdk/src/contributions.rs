//! Contributions (ADR-0034, decision 6): what a plugin adds to the core while
//! it is on, as entries of extension points.
//!
//! An extension point has a name, a major version, an entry type and a rule
//! for its entries' IDs. The points of SDK 1 are in
//! [`points`](crate::points), each named by a marker type, so a plugin builds
//! its contributions as `Contributions::new().add::<Tools>(tool)`. The core
//! module that owns a point consumes its entries, and the host hands them
//! over all or none: a plugin whose contributions the core refuses, or that
//! contributes to a point that no part of the core takes, fails to start
//! with `contributions_refused`.
//!
//! A plugin hands the host its contributions as it hands it trait objects
//! (decision 3), so the container itself never crosses JSON. The protocol
//! rules apply to each point's entries, and `tests/protocol.rs` lists the
//! points whose entries break them.

use std::any::{Any, TypeId};
use std::fmt;

/// An extension point: a kind of contribution, which one module of the core
/// consumes.
pub trait ExtensionPoint: 'static {
    /// The point's name, such as `tools`, unique among the points.
    const NAME: &'static str;
    /// The point's major version. An additive change keeps it (decision 14).
    const MAJOR: u16;
    /// The rule that the IDs of the point's entries follow.
    const IDS: IdRule;
    /// What a plugin contributes to the point.
    type Entry: Clone + Send + Sync + 'static;

    /// The ID of an entry, which the rule [`IDS`](Self::IDS) applies to.
    fn id(entry: &Self::Entry) -> &str;
}

/// The rule that the IDs of a point's entries follow (decision 6). The host
/// checks `Owned` and `PluginKind`, and the point's consumer checks that a
/// `Global` ID is free among the core's entries and every other plugin's.
///
/// The enum stays exhaustive, because the SDK's points produce its values
/// and the host matches them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdRule {
    /// The plugin's ID, or the plugin's ID, a dot and more, as
    /// [`owned_by`](crate::ids::owned_by) checks: mention providers and image
    /// sources.
    Owned,
    /// Exactly the plugin's ID, so a plugin has at most one entry: task
    /// executors.
    PluginKind,
    /// Unique among the core's entries and every plugin's: tools, agents and
    /// instruction sections.
    Global,
}

/// What the host knows of a point without its types: its name, its major
/// version and the rule of its IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PointInfo {
    /// The point's name.
    pub name: &'static str,
    /// The point's major version.
    pub major: u16,
    /// The rule of the point's IDs.
    pub ids: IdRule,
}

impl PointInfo {
    /// What a point declares of itself.
    pub const fn of<P: ExtensionPoint>() -> Self {
        Self {
            name: P::NAME,
            major: P::MAJOR,
            ids: P::IDS,
        }
    }
}

/// A plugin's entries for the points that it contributes to, in the order in
/// which it added them.
#[derive(Default)]
pub struct Contributions {
    entries: Vec<Contribution>,
}

/// One entry, with what the host needs to know of it without its type.
struct Contribution {
    point: PointInfo,
    /// The point's marker type, which keeps the entries of two points apart
    /// even when the points share an entry type.
    #[cfg_attr(
        not(feature = "host"),
        expect(
            dead_code,
            reason = "only the host reads entries back, behind the feature host; a plugin only adds them"
        )
    )]
    marker: TypeId,
    /// The entry's ID, read once when the plugin added the entry.
    id: String,
    /// A `P::Entry` of the point whose marker is `marker`.
    #[cfg_attr(
        not(feature = "host"),
        expect(
            dead_code,
            reason = "only the host reads entries back, behind the feature host; a plugin only adds them"
        )
    )]
    entry: Box<dyn Any + Send + Sync>,
}

impl Contributions {
    /// No contributions.
    pub fn new() -> Self {
        Self::default()
    }

    /// The contributions with one more entry of the point `P`.
    #[expect(
        clippy::should_implement_trait,
        reason = "`add::<P>` names its point by a type parameter, which `std::ops::Add::add` cannot take"
    )]
    #[must_use]
    pub fn add<P: ExtensionPoint>(mut self, entry: P::Entry) -> Self {
        self.entries.push(Contribution {
            point: PointInfo::of::<P>(),
            marker: TypeId::of::<P>(),
            id: P::id(&entry).to_owned(),
            entry: Box::new(entry),
        });
        self
    }
}

#[cfg(feature = "host")]
impl Contributions {
    /// The entries of the point `P`, in the order in which they were added.
    pub fn entries<P: ExtensionPoint>(&self) -> impl Iterator<Item = &P::Entry> {
        // `add` stores a `P::Entry` under the marker of `P`, so the downcast
        // of a matching entry always succeeds; one that did not would be
        // skipped rather than panic.
        self.entries
            .iter()
            .filter(|contribution| contribution.marker == TypeId::of::<P>())
            .filter_map(|contribution| contribution.entry.downcast_ref::<P::Entry>())
    }

    /// Takes the entries of the point `P` out of the contributions, in the
    /// order in which they were added.
    pub fn take<P: ExtensionPoint>(&mut self) -> Vec<P::Entry> {
        let (taken, kept): (Vec<Contribution>, Vec<Contribution>) =
            std::mem::take(&mut self.entries)
                .into_iter()
                .partition(|contribution| contribution.marker == TypeId::of::<P>());
        self.entries = kept;
        taken
            .into_iter()
            .filter_map(|contribution| contribution.entry.downcast::<P::Entry>().ok())
            .map(|entry| *entry)
            .collect()
    }

    /// Takes the entries of `points` out of the contributions, as the
    /// contributions that a consumer of those points gets.
    #[must_use]
    pub fn split_off(&mut self, points: &[PointInfo]) -> Self {
        let (taken, kept): (Vec<Contribution>, Vec<Contribution>) =
            std::mem::take(&mut self.entries)
                .into_iter()
                .partition(|contribution| points.contains(&contribution.point));
        self.entries = kept;
        Self { entries: taken }
    }

    /// The point and the ID of each entry, in the order in which they were
    /// added.
    pub fn ids(&self) -> impl Iterator<Item = (PointInfo, &str)> {
        self.entries
            .iter()
            .map(|contribution| (contribution.point, contribution.id.as_str()))
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl fmt::Debug for Contributions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The entries' own types need not implement Debug, so each entry
        // shows as its point and its ID.
        formatter
            .debug_list()
            .entries(
                self.entries
                    .iter()
                    .map(|contribution| (contribution.point.name, contribution.id.as_str())),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{Contributions, ExtensionPoint, IdRule, PointInfo};

    /// A point whose entries are names.
    enum Names {}

    impl ExtensionPoint for Names {
        const NAME: &'static str = "names";
        const MAJOR: u16 = 1;
        const IDS: IdRule = IdRule::Global;
        type Entry = String;

        fn id(entry: &String) -> &str {
            entry
        }
    }

    /// A second point whose entries are names too, which the first must not
    /// see.
    enum Labels {}

    impl ExtensionPoint for Labels {
        const NAME: &'static str = "labels";
        const MAJOR: u16 = 2;
        const IDS: IdRule = IdRule::Owned;
        type Entry = String;

        fn id(entry: &String) -> &str {
            entry
        }
    }

    #[test]
    fn a_point_declares_its_name_major_and_rule() {
        let info = PointInfo::of::<Labels>();

        assert_eq!(info.name, "labels");
        assert_eq!(info.major, 2);
        assert_eq!(info.ids, IdRule::Owned);
    }

    #[test]
    fn contributions_show_each_entry_as_its_point_and_its_id() {
        let contributions = Contributions::new()
            .add::<Names>("alpha".to_owned())
            .add::<Labels>("fake.label".to_owned());

        assert_eq!(
            format!("{contributions:?}"),
            r#"[("names", "alpha"), ("labels", "fake.label")]"#
        );
        assert_eq!(format!("{:?}", Contributions::new()), "[]");
    }

    #[cfg(feature = "host")]
    mod host {
        use super::{Labels, Names};
        use crate::contributions::{Contributions, PointInfo};

        fn mixed() -> Contributions {
            Contributions::new()
                .add::<Names>("alpha".to_owned())
                .add::<Labels>("fake.label".to_owned())
                .add::<Names>("beta".to_owned())
        }

        #[test]
        fn entries_keep_their_point_and_their_order_even_when_points_share_a_type() {
            let contributions = mixed();

            let names: Vec<&String> = contributions.entries::<Names>().collect();
            let labels: Vec<&String> = contributions.entries::<Labels>().collect();
            let ids: Vec<(&str, &str)> = contributions
                .ids()
                .map(|(point, id)| (point.name, id))
                .collect();

            assert_eq!(names, ["alpha", "beta"]);
            assert_eq!(labels, ["fake.label"]);
            assert_eq!(
                ids,
                [
                    ("names", "alpha"),
                    ("labels", "fake.label"),
                    ("names", "beta")
                ]
            );
            assert!(!contributions.is_empty());
            assert!(Contributions::new().is_empty());
        }

        #[test]
        fn taking_a_points_entries_leaves_the_others() {
            let mut contributions = mixed();

            let names = contributions.take::<Names>();
            let none = contributions.take::<Names>();

            assert_eq!(names, ["alpha", "beta"]);
            assert_eq!(none, Vec::<String>::new());
            let left: Vec<&String> = contributions.entries::<Labels>().collect();
            assert_eq!(left, ["fake.label"]);
        }

        #[test]
        fn splitting_off_points_moves_their_entries_and_keeps_the_rest() {
            let mut contributions = mixed();

            let names = contributions.split_off(&[PointInfo::of::<Names>()]);
            let nothing = contributions.split_off(&[]);

            let split: Vec<&String> = names.entries::<Names>().collect();
            let kept: Vec<(&str, &str)> = contributions
                .ids()
                .map(|(point, id)| (point.name, id))
                .collect();
            assert_eq!(split, ["alpha", "beta"]);
            assert_eq!(names.entries::<Labels>().count(), 0);
            assert_eq!(kept, [("labels", "fake.label")]);
            assert!(nothing.is_empty());
        }
    }
}
