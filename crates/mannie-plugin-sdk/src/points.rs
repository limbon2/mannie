//! The extension points that plugins contribute to (ADR-0034, decision 6).
//! Each module holds one point's marker type and entry types, and the core
//! module that owns the point consumes them, so the runtime's `images.rs`
//! consumes image sources, its `mentions.rs` mention providers, its
//! `agents.rs` agent specs, its `instructions.rs` sections, its
//! `executors.rs` task executors and its `plugin_tools.rs` tools.
//!
//! SDK 1 moves today's points in their current shapes (decision 3). Where a
//! shape breaks the protocol rules, `tests/protocol.rs` lists the module as a
//! known exception with its reason.

pub mod agents;
pub mod executors;
pub mod images;
pub mod mentions;
pub mod sections;
pub mod tools;

use crate::contributions::PointInfo;

/// Every point of the SDK. The host refuses a plugin that contributes to a
/// point that no consumer takes, and the core's tests check that its
/// consumers take each of these.
pub const ALL: &[PointInfo] = &[
    PointInfo::of::<tools::Tools>(),
    PointInfo::of::<agents::Agents>(),
    PointInfo::of::<sections::Sections>(),
    PointInfo::of::<executors::Executors>(),
    PointInfo::of::<mentions::Mentions>(),
    PointInfo::of::<images::ImageSources>(),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::ALL;
    use crate::contributions::IdRule;

    #[test]
    fn every_point_has_a_name_of_its_own_and_the_rule_of_its_ids() {
        let names: BTreeSet<&str> = ALL.iter().map(|point| point.name).collect();
        let rules: Vec<(&str, u16, IdRule)> = ALL
            .iter()
            .map(|point| (point.name, point.major, point.ids))
            .collect();

        assert_eq!(names.len(), ALL.len());
        assert_eq!(
            rules,
            [
                ("tools", 1, IdRule::Global),
                ("agents", 1, IdRule::Global),
                ("sections", 1, IdRule::Global),
                ("executors", 1, IdRule::PluginKind),
                ("mentions", 1, IdRule::Owned),
                ("image-sources", 1, IdRule::Owned),
            ]
        );
    }
}
