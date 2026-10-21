//! Versions and feature levels (plugins/workers/docs/
//! compatibility-and-updates.md): the master works with a worker of its own
//! minor version or the one before it, and each package of the link has a
//! feature level, of which both sides use the lower.

use std::collections::HashMap;

/// The feature level that this version speaks of each package of the link.
/// Every package of `plugins/workers/proto/` has one, which a test checks.
pub const LEVELS: [(&str, u32); 6] = [
    ("mannie.link.session.v1", 1),
    ("mannie.link.pairing.v1", 1),
    ("mannie.link.tasks.v1", 1),
    ("mannie.link.events.v1", 2),
    ("mannie.link.approvals.v1", 1),
    ("mannie.link.leases.v1", 1),
];

/// This version's levels, as `Hello` carries them.
pub fn levels() -> HashMap<String, u32> {
    LEVELS
        .iter()
        .map(|(package, level)| ((*package).to_owned(), *level))
        .collect()
}

/// The levels that both sides speak: the lower of each package. A package
/// that the other side does not list is missing from the result.
pub fn agree(theirs: &HashMap<String, u32>) -> HashMap<String, u32> {
    LEVELS
        .iter()
        .filter_map(|(package, ours)| {
            let level = (*ours).min(*theirs.get(*package)?);
            (level > 0).then(|| ((*package).to_owned(), level))
        })
        .collect()
}

/// Whether the agreed levels cover every package that this version uses.
pub fn covers_every_package(agreed: &HashMap<String, u32>) -> bool {
    LEVELS
        .iter()
        .all(|(package, _level)| agreed.get(*package).is_some_and(|level| *level > 0))
}

/// A version of Mannie: its major, minor and patch numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// The major number.
    pub major: u32,
    /// The minor number.
    pub minor: u32,
    /// The patch number.
    pub patch: u32,
}

impl Version {
    /// A version such as `0.4.1`, without a pre-release or build part; `None`
    /// for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let mut numbers = text.trim().split('.').map(str::parse::<u32>);
        let version = Self {
            major: numbers.next()?.ok()?,
            minor: numbers.next()?.ok()?,
            patch: numbers.next()?.ok()?,
        };
        numbers.next().is_none().then_some(version)
    }
}

/// Whether a master of version `master` works with a worker of version
/// `worker`: the same major and minor version, or the minor version before;
/// patch versions do not matter. A worker newer than the master needs a
/// newer master.
pub fn compatible(master: Version, worker: Version) -> bool {
    master.major == worker.major
        && (worker.minor == master.minor || worker.minor.checked_add(1) == Some(master.minor))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{LEVELS, Version, agree, compatible, covers_every_package, levels};

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn a_worker_one_minor_version_behind_works_and_others_do_not() {
        let master = version("0.5.2");

        assert!(compatible(master, version("0.5.0")));
        assert!(compatible(master, version("0.5.9")));
        assert!(compatible(master, version("0.4.7")));
        assert!(!compatible(master, version("0.3.9")), "older");
        assert!(!compatible(master, version("0.6.0")), "newer");
        assert!(!compatible(master, version("1.5.2")), "another major");
        assert!(!compatible(version("0.0.4"), version("1.0.4")));
        assert_eq!(Version::parse("0.4"), None);
        assert_eq!(Version::parse("0.4.1-rc.1"), None);
        assert_eq!(Version::parse("0.4.1.2"), None);
    }

    #[test]
    fn both_sides_use_the_lower_level_of_each_package() {
        let mut theirs = levels();
        theirs.insert("mannie.link.tasks.v1".to_owned(), 3);
        theirs.insert("mannie.link.future.v1".to_owned(), 1);

        let agreed = agree(&theirs);

        assert_eq!(agreed.get("mannie.link.tasks.v1"), Some(&1));
        assert!(!agreed.contains_key("mannie.link.future.v1"));
        assert!(covers_every_package(&agreed));
        let mut missing = levels();
        missing.remove("mannie.link.leases.v1");
        assert!(!covers_every_package(&agree(&missing)));
        assert!(!covers_every_package(&agree(&HashMap::new())));
    }

    #[test]
    fn every_package_of_the_contracts_has_a_level() {
        let proto =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../proto/mannie/link");
        let mut packages: Vec<String> = std::fs::read_dir(&proto)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .map(|service| format!("mannie.link.{service}.v1"))
            .collect();
        packages.sort();
        let mut listed: Vec<String> = LEVELS
            .iter()
            .map(|(package, _)| (*package).to_owned())
            .collect();
        listed.sort();

        assert_eq!(packages, listed);
    }
}
