//! The installed git's version (plugins/git/docs/adr/0001-run-the-owners-
//! installed-git.md): the plugin needs git 2.40 or later, the first with
//! `merge-tree --merge-base`, and reads the version from `git version`, such
//! as `git version 2.54.0.windows.1`.

use std::fmt;

/// A version of git: its major, minor and patch numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version {
    pub(crate) major: u32,
    pub(crate) minor: u32,
    pub(crate) patch: u32,
}

impl Version {
    /// The oldest git that the plugin runs.
    pub(crate) const MINIMUM: Self = Self::new(2, 40, 0);

    /// The first git that lists the hooks of an event, including the hooks
    /// that configuration defines, with `git hook list`.
    pub(crate) const HOOK_LIST: Self = Self::new(2, 54, 0);

    pub(crate) const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Reads the output of `git version`: the leading digits of its first
    /// three parts between dots, so builds such as `2.54.0.windows.1` or
    /// `2.39.3 (Apple Git-146)` read as their release, and a part without
    /// digits, such as `rc1`, as 0.
    pub(crate) fn parse(output: &str) -> Option<Self> {
        let rest = output.trim().strip_prefix("git version ")?;
        let mut parts = rest.split('.');
        let major = leading_number(parts.next()?)?;
        let minor = leading_number(parts.next()?)?;
        let patch = parts.next().and_then(leading_number).unwrap_or(0);
        Some(Self::new(major, minor, patch))
    }
}

/// The number that a text starts with, or `None`.
fn leading_number(part: &str) -> Option<u32> {
    let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::Version;

    #[test]
    fn the_version_of_each_build_reads_as_its_release() {
        for (output, version) in [
            ("git version 2.54.0.windows.1\n", Version::new(2, 54, 0)),
            ("git version 2.39.3 (Apple Git-146)", Version::new(2, 39, 3)),
            ("git version 2.40.0", Version::new(2, 40, 0)),
            ("git version 2.45.rc1", Version::new(2, 45, 0)),
            ("git version 3.0", Version::new(3, 0, 0)),
        ] {
            assert_eq!(Version::parse(output), Some(version), "{output}");
        }
        for output in ["", "git version", "hub version 2.14.2", "git version x.1"] {
            assert_eq!(Version::parse(output), None, "{output}");
        }
    }

    #[test]
    fn versions_order_by_their_numbers() {
        assert!(Version::new(2, 39, 9) < Version::MINIMUM);
        assert!(Version::new(2, 40, 0) >= Version::MINIMUM);
        assert!(Version::new(2, 100, 0) > Version::new(2, 54, 0));
        assert_eq!(Version::new(2, 54, 1).to_string(), "2.54.1");
    }
}
