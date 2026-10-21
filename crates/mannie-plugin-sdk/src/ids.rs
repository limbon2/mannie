//! Plugin IDs and the rules that keep them apart (ADR-0034, decision 10;
//! ADR-0030, decision 3).
//!
//! A plugin's ID names its folder and namespaces its settings section, its
//! commands (`plugin:<id>|`) and its secrets (`<id>/`). Its prefix, the ID
//! with underscores in place of hyphens, namespaces its tables and its
//! message keys as `<prefix>_*`.
//!
//! [`PluginId::parse`] makes the checks that one ID allows on its own: the
//! syntax, the names that Tauri, the core, the SDK, SQLite and Windows
//! reserve, the names of Tauri's plugin crates, and the prefixes of the
//! tables that are not a plugin's. Two rules need more than one ID, so other
//! parts check them. The host refuses two plugins whose prefixes nest, in the
//! sense of [`prefixes_nest`]. The build refuses a plugin whose prefix nests
//! with a prefix of the core's message keys, which it reads from the core's
//! message catalogs.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

/// IDs that no plugin may have.
pub const RESERVED_IDS: &[&str] = &[
    // Tauri's own plugins and the Tauri plugins that the desktop app adds.
    // Each Mannie plugin becomes a Tauri plugin under its ID (decision 8), and
    // a Tauri plugin whose name is taken replaces the earlier one without an
    // error (`PluginStore::register` in tauri 2.12.3). Tauri itself refuses
    // `core` and `tauri` (`RESERVED_PLUGIN_NAMES`).
    "app",
    "core",
    "event",
    "image",
    "menu",
    "opener",
    "path",
    "resources",
    "single-instance",
    "tauri",
    "tray",
    "webview",
    "window",
    // The core's table prefixes, the prefixes of the tables that validation
    // builds and the store's tests create, the desktop app's validation
    // commands, and the core's namespaces in the credential store: provider
    // credentials under `provider/` and Workers' legacy entry `link/master-key`.
    "link",
    "exports",
    "provider",
    "runtime",
    "settings",
    "store",
    "tests",
    "tools",
    "validation",
    // The core's settings section `interface`. The core's other sections are
    // reserved above or are prefixes of the core's message keys (`chat` and
    // `child_agents`), which the build checks.
    "interface",
    // SQLite keeps its own tables under `sqlite_`, such as `sqlite_schema`,
    // `sqlite_sequence` and `sqlite_stat1`, and the last two accept ordinary
    // writes. A plugin confined to `sqlite_*` would reach them, and no plugin
    // could make a table of its own there, because SQLite refuses
    // `CREATE TABLE sqlite_*`. `CORE_TABLE_PREFIXES` also refuses the IDs
    // whose prefixes nest with it, such as `sqlite-stat1`.
    "sqlite",
    // The device names that Windows refuses as folder names, so that the
    // folder `plugins/<id>/` can be checked out on Windows (ADR-0035,
    // decision 1). `com0` and `lpt0` are ordinary folder names on Windows 11
    // (checked on 2026-10-17).
    "aux",
    "com1",
    "com2",
    "com3",
    "com4",
    "com5",
    "com6",
    "com7",
    "com8",
    "com9",
    "con",
    "lpt1",
    "lpt2",
    "lpt3",
    "lpt4",
    "lpt5",
    "lpt6",
    "lpt7",
    "lpt8",
    "lpt9",
    "nul",
    "prn",
    // The names of Mannie, the SDK and the host.
    "host",
    "mannie",
    "plugin",
    "plugins",
    "sdk",
];

/// The prefixes, as `<prefix>_*`, of the tables in the store's database that
/// are not a plugin's: the core's, the core's `validation_*` of validation
/// builds, and SQLite's own `sqlite_*`. A plugin's prefix may not begin one
/// of them, and none of them may begin a plugin's, so that the statements of
/// a plugin confined to its own prefix never reach such a table (decision 7).
/// The store's tests also create `tests_*` tables, which no shipped or
/// validation database holds, so `tests` is only a reserved ID.
pub const CORE_TABLE_PREFIXES: &[&str] = &[
    "settings",
    "runtime",
    "exports",
    "provider",
    "tools",
    "store",
    "validation",
    "sqlite",
];

/// The start of the crate names of Tauri's plugins, which no plugin ID may
/// share.
pub const TAURI_PLUGIN_CRATE_PREFIX: &str = "tauri-plugin-";

/// The syntax of a plugin ID as a regular expression, the same rule that
/// [`PluginId::parse`] checks first.
const PLUGIN_ID_PATTERN: &str = "^[a-z][a-z0-9]*(-[a-z0-9]+)*$";

/// How much of a refused ID an error repeats. An ID can come from the
/// webview, which is untrusted, and errors reach the logs.
const MAX_ECHOED_LENGTH: usize = 64;

/// The ID of a plugin: lower-case words of letters and digits joined by
/// single hyphens, starting with a letter, such as `workers` or
/// `log-viewer`. It equals the name of the plugin's folder.
///
/// It crosses JSON as its text, and decoding checks the same rules as
/// [`PluginId::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId(String);

impl PluginId {
    /// Checks an ID against the syntax, the reserved IDs, the names of Tauri's
    /// plugin crates and the prefixes of the tables that are not a plugin's,
    /// in that order.
    pub fn parse(id: &str) -> Result<Self, IdError> {
        if !has_plugin_id_syntax(id) {
            return Err(IdError::Syntax { id: echoed(id) });
        }
        if RESERVED_IDS.contains(&id) {
            return Err(IdError::Reserved { id: echoed(id) });
        }
        if id.starts_with(TAURI_PLUGIN_CRATE_PREFIX) {
            return Err(IdError::TauriPrefix { id: echoed(id) });
        }
        let prefix = prefix_of(id);
        let nested_core = CORE_TABLE_PREFIXES
            .iter()
            .copied()
            .find(|core| prefixes_nest(&prefix, core));
        if let Some(core) = nested_core {
            return Err(IdError::NestsCorePrefix {
                id: echoed(id),
                core,
            });
        }
        Ok(Self(id.to_owned()))
    }

    /// The ID as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The prefix of the plugin's tables and message keys: the ID with
    /// underscores in place of hyphens, without the underscore that follows
    /// it in a name.
    pub fn prefix(&self) -> String {
        prefix_of(&self.0)
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for PluginId {
    type Error = IdError;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        Self::parse(&id)
    }
}

impl From<PluginId> for String {
    fn from(id: PluginId) -> Self {
        id.0
    }
}

/// The schema of an ID in a plugin's manifest, which editors and Tombi check
/// as the owner types it. It states the syntax alone: the reserved IDs, the
/// names of Tauri's plugin crates and the table prefixes are refused when the
/// manifest is parsed.
impl JsonSchema for PluginId {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("PluginId")
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed(concat!(module_path!(), "::PluginId"))
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": PLUGIN_ID_PATTERN,
        })
    }
}

/// Why a text is not a plugin ID. Each variant repeats the refused ID, cut to
/// its first 64 characters.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IdError {
    /// The text is not lower-case words of letters and digits joined by
    /// single hyphens, starting with a letter.
    #[error(
        "{id:?} is not a plugin ID, which is lower-case words of letters and digits joined by single hyphens, starting with a letter"
    )]
    Syntax {
        /// The refused ID.
        id: String,
    },
    /// The ID names Tauri's own plugins or Tauri plugins of the desktop app,
    /// a namespace of the core, SQLite's tables, a Windows device, or Mannie,
    /// the SDK or the host.
    #[error(
        "{id:?} is reserved, because Tauri, the core, the SDK, SQLite or Windows uses the name"
    )]
    Reserved {
        /// The refused ID.
        id: String,
    },
    /// The ID starts with `tauri-plugin-`, as the crates of Tauri's plugins
    /// do.
    #[error("{id:?} starts with \"tauri-plugin-\", as the crates of Tauri's plugins do")]
    TauriPrefix {
        /// The refused ID.
        id: String,
    },
    /// The plugin's prefix and one of [`CORE_TABLE_PREFIXES`] begin one
    /// another, so the plugin's tables could not be told apart from tables
    /// that are not a plugin's.
    #[error(
        "the tables of {id:?} would share a prefix with the tables named {core}_*, which are not a plugin's"
    )]
    NestsCorePrefix {
        /// The refused ID.
        id: String,
        /// The table prefix that the plugin's prefix nests with.
        core: &'static str,
    },
}

/// Whether two prefixes nest: whether `<a>_` begins `<b>_`, or `<b>_` begins
/// `<a>_`. Equal prefixes nest. The plugin `log` nests with `log-viewer`,
/// because tables named `log_*` include `log_viewer_*`, and `image` does not
/// nest with `images`.
pub fn prefixes_nest(a: &str, b: &str) -> bool {
    let a = format!("{a}_");
    let b = format!("{b}_");
    a.starts_with(&b) || b.starts_with(&a)
}

/// Whether an entry's ID belongs to a plugin under the rule `Owned` of
/// extension points, such as the IDs of mention providers and image sources:
/// the plugin's ID itself, or the plugin's ID, a dot and more (decision 6).
pub fn owned_by(plugin: &str, id: &str) -> bool {
    id == plugin
        || id
            .strip_prefix(plugin)
            .is_some_and(|rest| rest.len() > 1 && rest.starts_with('.'))
}

/// Whether a text is lower-case words of letters and digits joined by single
/// hyphens, starting with a letter.
fn has_plugin_id_syntax(id: &str) -> bool {
    let starts_with_letter = id
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase());
    starts_with_letter
        && id.split('-').all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        })
}

/// The prefix of an ID that has the syntax of a plugin ID.
fn prefix_of(id: &str) -> String {
    id.replace('-', "_")
}

/// The part of a refused ID that an error repeats.
fn echoed(id: &str) -> String {
    id.chars().take(MAX_ECHOED_LENGTH).collect()
}

#[cfg(test)]
mod tests {
    use super::{CORE_TABLE_PREFIXES, IdError, PluginId, RESERVED_IDS, owned_by, prefixes_nest};

    #[test]
    fn plugin_ids_are_lower_case_words_joined_by_hyphens() {
        for valid in ["workers", "images", "log-viewer", "jira2", "a-b-c"] {
            let id = PluginId::parse(valid).unwrap();
            assert_eq!(id.as_str(), valid);
            assert_eq!(id.to_string(), valid);
        }
        for invalid in [
            "",
            "Workers",
            "log_viewer",
            "-workers",
            "workers-",
            "log--viewer",
            "2fa",
            "a b",
            "fake.notes",
            "wörkers",
        ] {
            assert_eq!(
                PluginId::parse(invalid),
                Err(IdError::Syntax {
                    id: invalid.to_owned()
                }),
                "{invalid}"
            );
        }
    }

    #[test]
    fn reserved_ids_are_refused_after_the_syntax_check() {
        for reserved in RESERVED_IDS {
            // A reserved ID that broke the syntax would never be reached, so
            // each one must fail as reserved and not as syntax.
            assert_eq!(
                PluginId::parse(reserved),
                Err(IdError::Reserved {
                    id: (*reserved).to_owned()
                }),
                "{reserved}"
            );
        }
    }

    #[test]
    fn every_core_table_prefix_is_a_reserved_id() {
        for core in CORE_TABLE_PREFIXES {
            assert!(RESERVED_IDS.contains(core), "{core}");
        }
    }

    #[test]
    fn ids_that_start_like_a_tauri_plugin_crate_are_refused() {
        for id in ["tauri-plugin-x", "tauri-plugin-fs"] {
            assert_eq!(
                PluginId::parse(id),
                Err(IdError::TauriPrefix { id: id.to_owned() }),
                "{id}"
            );
        }
        assert_eq!(PluginId::parse("tauri-x").unwrap().as_str(), "tauri-x");
    }

    #[test]
    fn ids_whose_tables_nest_with_a_core_prefix_are_refused() {
        for (id, core) in [
            ("exports-private", "exports"),
            ("runtime-x", "runtime"),
            ("settings-extra", "settings"),
            ("provider-keys", "provider"),
            ("tools-git", "tools"),
            ("store-files", "store"),
            ("validation-foo", "validation"),
        ] {
            assert_eq!(
                PluginId::parse(id),
                Err(IdError::NestsCorePrefix {
                    id: id.to_owned(),
                    core
                }),
                "{id}"
            );
        }
        // A longer word is another prefix: `runtimes_*` is not `runtime_*`.
        for apart in ["runtimes", "tool", "stores", "settings2", "validations"] {
            assert_eq!(PluginId::parse(apart).unwrap().as_str(), apart);
        }
    }

    #[test]
    fn ids_whose_tables_would_reach_sqlites_own_tables_are_refused() {
        // Under the prefix `sqlite`, the authorizer would admit
        // `sqlite_sequence` and `sqlite_stat1`, which accept writes.
        assert_eq!(
            PluginId::parse("sqlite"),
            Err(IdError::Reserved {
                id: "sqlite".to_owned()
            })
        );
        for id in [
            "sqlite-stat1",
            "sqlite-sequence",
            "sqlite-x",
            "sqlite-autoindex-runtime",
        ] {
            assert_eq!(
                PluginId::parse(id),
                Err(IdError::NestsCorePrefix {
                    id: id.to_owned(),
                    core: "sqlite"
                }),
                "{id}"
            );
        }
        assert_eq!(PluginId::parse("sqlites").unwrap().as_str(), "sqlites");
    }

    #[test]
    fn windows_device_names_are_refused_and_longer_names_are_not() {
        for device in ["con", "prn", "aux", "nul", "com1", "com9", "lpt1", "lpt9"] {
            assert_eq!(
                PluginId::parse(device),
                Err(IdError::Reserved {
                    id: device.to_owned()
                }),
                "{device}"
            );
        }
        // Windows treats only the exact names as devices.
        for ordinary in [
            "com0",
            "lpt0",
            "com10",
            "console",
            "null",
            "aux-tools",
            "nul2",
        ] {
            assert_eq!(PluginId::parse(ordinary).unwrap().as_str(), ordinary);
        }
    }

    #[test]
    fn the_cores_interface_settings_section_is_a_reserved_id() {
        assert_eq!(
            PluginId::parse("interface"),
            Err(IdError::Reserved {
                id: "interface".to_owned()
            })
        );
    }

    #[test]
    fn an_error_repeats_at_most_64_characters_of_the_refused_id() {
        let long = "X".repeat(10_000);
        let Err(IdError::Syntax { id }) = PluginId::parse(&long) else {
            panic!("a long upper-case ID breaks the syntax");
        };
        assert_eq!(id, "X".repeat(64));

        let long_valid_syntax = format!("runtime-{}", "a".repeat(10_000));
        let Err(IdError::NestsCorePrefix { id, core }) = PluginId::parse(&long_valid_syntax) else {
            panic!("a long runtime- ID nests with the runtime's tables");
        };
        assert_eq!(id.len(), 64);
        assert_eq!(core, "runtime");
    }

    #[test]
    fn errors_name_the_refused_id_and_the_rule() {
        let syntax = PluginId::parse("Workers").unwrap_err().to_string();
        assert!(syntax.contains("\"Workers\""), "{syntax}");
        assert!(syntax.contains("lower-case words"), "{syntax}");

        let reserved = PluginId::parse("plugins").unwrap_err().to_string();
        assert!(reserved.contains("\"plugins\" is reserved"), "{reserved}");

        let nested = PluginId::parse("tools-git").unwrap_err().to_string();
        assert!(nested.contains("\"tools-git\""), "{nested}");
        assert!(nested.contains("tools_*"), "{nested}");

        let nested_sqlite = PluginId::parse("sqlite-stat1").unwrap_err().to_string();
        assert!(
            nested_sqlite.contains("\"sqlite-stat1\""),
            "{nested_sqlite}"
        );
        assert!(nested_sqlite.contains("sqlite_*"), "{nested_sqlite}");
    }

    #[test]
    fn the_prefix_replaces_hyphens_with_underscores() {
        assert_eq!(PluginId::parse("workers").unwrap().prefix(), "workers");
        assert_eq!(
            PluginId::parse("log-viewer").unwrap().prefix(),
            "log_viewer"
        );
        assert_eq!(PluginId::parse("a-b-c").unwrap().prefix(), "a_b_c");
    }

    #[test]
    fn prefixes_nest_when_one_begins_the_other_at_a_word() {
        assert!(prefixes_nest("log", "log_viewer"));
        assert!(prefixes_nest("log_viewer", "log"));
        assert!(prefixes_nest("workers", "workers"));
        assert!(prefixes_nest("child", "child_agents"));
        assert!(!prefixes_nest("image", "images"));
        assert!(!prefixes_nest("images", "image"));
        assert!(!prefixes_nest("log", "logs_viewer"));
        assert!(!prefixes_nest("workers", "images"));
    }

    #[test]
    fn an_owned_id_is_the_plugins_id_or_starts_with_it_and_a_dot() {
        assert!(owned_by("fake", "fake"));
        assert!(owned_by("fake", "fake.notes"));
        assert!(owned_by("fake", "fake.notes.recent"));
        assert!(!owned_by("fake", "fakeish"));
        assert!(!owned_by("fake", "fake."));
        assert!(!owned_by("fake", "other.notes"));
        assert!(!owned_by("fake", "notes.fake"));
        assert!(!owned_by("fake", ""));
    }
}
