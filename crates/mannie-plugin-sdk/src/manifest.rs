//! A plugin's manifest, `plugins/<id>/plugin.toml` (ADR-0035, decision 1):
//! the plugin's ID, the SDK major that it builds on, its order, its entry
//! crate, the services that it uses, its secret entries, the third-party
//! crates that only it may use, its programs and its direct native calls.
//!
//! The manifest is the reviewed registration of a plugin, so the host grants
//! the plugin the services that it declares and nothing else (ADR-0034,
//! decisions 5 and 13). Each field has a reader: the host reads the ID, the
//! major, the order, the services and the secrets, and shows the names of
//! the programs, the crates and the native calls on the Plugins page; the
//! build's generator reads the entry crate (ADR-0035, decision 3), the
//! build's boundary check reads the crates (decision 6), the build, CI and
//! the release read the programs (decision 10), and the lint's Clippy run
//! reads the native calls (decision 6). A field that nothing reads is not
//! part of the manifest, and `Manifest::parse`, behind the feature
//! `manifest`, refuses it.
//!
//! The SDK commits the manifest's JSON Schema as `plugin.schema.json`, which
//! a manifest names in its first line, `#:schema`, so Tombi and editors check
//! it as the owner types it.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;

use crate::ids::PluginId;

/// A plugin's manifest, as `plugin.toml` holds it.
///
/// A plugin's code never sees it: the plugin's entry crate embeds the text,
/// and the host parses and checks it before it starts the plugin.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[non_exhaustive]
pub struct Manifest {
    /// The plugin's ID, which equals the name of its folder, `plugins/<id>/`.
    pub id: PluginId,
    /// The major version of the plugin SDK that the plugin builds on. The
    /// host refuses a plugin for another major with the code
    /// `sdk_incompatible`.
    pub sdk: u16,
    /// Where the plugin starts and where the Plugins page lists it: the
    /// lowest first, and plugins with the same order by their IDs.
    pub order: u32,
    /// The plugin's Rust code.
    pub rust: RustSection,
    /// The services that the plugin uses, each with the major version that
    /// it builds on, such as `executor-host = 1`. The host binds these and
    /// no other.
    #[serde(default)]
    pub services: BTreeMap<String, u16>,
    /// The plugin's entries in the credential store, which the service
    /// `secrets` reaches.
    #[serde(default)]
    pub secrets: SecretsSection,
    /// The third-party crates that only this plugin may use.
    #[serde(default)]
    pub crates: CratesSection,
    /// The programs that ship beside the desktop app, built from the
    /// plugin's workspace, as `[[programs]]` tables.
    #[serde(default)]
    pub programs: Vec<ProgramSection>,
    /// The direct native calls that the plugin's library crates make past
    /// the SDK's services.
    #[serde(default)]
    pub native: NativeSection,
}

/// The `[rust]` table of a manifest: the plugin's Rust code.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[non_exhaustive]
pub struct RustSection {
    /// The package in the plugin's folder that calls `export_plugin!`, such
    /// as `mannie-workers`.
    pub entry: String,
}

/// The `[secrets]` table of a manifest: the names of the plugin's entries in
/// the credential store. A name is words of ASCII letters, digits, `.`, `_`
/// and `-`, joined by single slashes. A name may appear twice in one list,
/// because the store keys an entry by its name and both name the same
/// entry, but it may not appear in both lists, and no entry or legacy name
/// may be a keyed family or an entry of one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[non_exhaustive]
pub struct SecretsSection {
    /// Entries that the store keeps as `<id>/<entry>`, so a plugin reaches
    /// only its own (ADR-0034, decision 10).
    #[serde(default)]
    pub entries: Vec<String>,
    /// Names from before plugins had their own entries, which the store
    /// keeps as they are. The core's credential provider accepts only the
    /// names of its frozen list, which holds Workers' `link/master-key`.
    #[serde(default)]
    pub legacy: Vec<String>,
    /// Families of entries that the plugin creates and deletes as it needs
    /// them, such as a password for each host that the owner registers
    /// (ADR-0036, decision 8). The plugin names an entry `<family>/<key>`,
    /// whose key has 1 to 64 ASCII letters, digits and `-`, and the store
    /// keeps it as `<id>/<family>/<key>`.
    #[serde(default)]
    pub keyed: Vec<String>,
}

/// The most characters of the key of an entry of a keyed family.
pub const MAX_SECRET_KEY_CHARS: usize = 64;

impl SecretsSection {
    /// The name under which the credential store keeps the entry `entry` of
    /// the plugin `plugin`, or `None` when this section does not give the
    /// plugin that entry: an entry of `entries` is kept as `<id>/<entry>`, a
    /// legacy name as it is, and `<family>/<key>` of a keyed family as
    /// `<id>/<family>/<key>`.
    pub fn stored_name(&self, plugin: &str, entry: &str) -> Option<String> {
        if self.entries.iter().any(|listed| listed == entry) {
            return Some(format!("{plugin}/{entry}"));
        }
        if self.legacy.iter().any(|listed| listed == entry) {
            return Some(entry.to_owned());
        }
        let (family, key) = entry.rsplit_once('/')?;
        let is_family = self.keyed.iter().any(|listed| listed == family);
        (is_family && is_secret_key(key)).then(|| format!("{plugin}/{entry}"))
    }
}

/// Whether `key` can name an entry of a keyed family: 1 to 64 ASCII letters,
/// digits and `-`, which the credential store accepts in a name and which
/// keep a key from adding a slash, such as the ID of a host.
pub fn is_secret_key(key: &str) -> bool {
    let has_valid_length = (1..=MAX_SECRET_KEY_CHARS).contains(&key.len());
    has_valid_length
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// The `[crates]` table of a manifest: the crates from a registry that only
/// this plugin may use. The build's boundary check refuses every direct
/// dependency on one of them from outside the plugin's folder, except from
/// another crate of the same list, and two plugins that list one crate
/// (ADR-0035, decision 6). The host only shows the names on the Plugins
/// page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[non_exhaustive]
pub struct CratesSection {
    /// The crates' names, such as `tonic`.
    #[serde(default)]
    pub exclusive: Vec<String>,
}

/// The `[native]` table of a manifest: the categories of direct native calls
/// that the plugin's library crates make past the SDK's services, each with
/// its reason, such as the sockets of Workers' link. Clippy refuses the
/// calls of every other category in the library crates that the plugin's
/// entry crate ships, and `task plugins:grants` lists the grants (ADR-0035,
/// decision 6). The host only shows the categories on the Plugins page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[non_exhaustive]
pub struct NativeSection {
    /// Why the library crates write files themselves.
    #[serde(default)]
    pub file_writes: Option<String>,
    /// Why the library crates start or end processes themselves.
    #[serde(default)]
    pub processes: Option<String>,
    /// Why the library crates open sockets themselves.
    #[serde(default)]
    pub sockets: Option<String>,
}

impl NativeSection {
    /// The categories that the table grants, each with its reason, in the
    /// order of the table's keys.
    pub fn grants(&self) -> Vec<(&'static str, &str)> {
        [
            ("file-writes", &self.file_writes),
            ("processes", &self.processes),
            ("sockets", &self.sockets),
        ]
        .into_iter()
        .filter_map(|(category, reason)| reason.as_deref().map(|reason| (category, reason)))
        .collect()
    }
}

/// A `[[programs]]` table of a manifest: a program of the plugin's workspace
/// that ships beside the desktop app, such as Workers' worker service. The
/// build, the end-to-end tests, CI and the release iterate over these
/// (ADR-0035, decision 10). The host only shows the package on the Plugins
/// page.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[non_exhaustive]
pub struct ProgramSection {
    /// The program's package in the plugin's workspace, whose binary has the
    /// same name, such as `mannie-worker`.
    pub package: String,
    /// The library crates of the plugin that only the program uses, such as
    /// the worker's side of Workers' link. They and the program may depend
    /// on the host kit, which the plugin's entry crate never reaches
    /// (ADR-0034, decision 2).
    #[serde(default)]
    pub crates: Vec<String>,
    /// Whether the program runs without a window, so that no crate of
    /// Tauri, GTK or WebKitGTK may reach it on any target (V5).
    #[serde(default)]
    pub headless: bool,
    /// The targets that a release builds the program for, such as
    /// `x86_64-unknown-freebsd`.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Whether the release signs the program's archives. A signed program
    /// embeds the release key: its `self-check` reports the key's SHA-256
    /// as `releaseKeySha256`, and `update --check <archive>` checks an
    /// archive's signature with it.
    #[serde(default)]
    pub signed: bool,
    /// The arguments with which the release runs the built program to
    /// check it, such as `["self-check", "--json"]`.
    #[serde(default)]
    pub self_check: Vec<String>,
    /// Whether `task e2e` builds the program for the end-to-end tests.
    #[serde(default)]
    pub e2e: bool,
    /// The program's smoke tests in CI.
    #[serde(default)]
    pub smoke: SmokeSection,
}

/// The `smoke` table of a program: scripts in the plugin's folder that CI
/// runs as root or elevated on a disposable machine. CI first builds the
/// program in the debug profile, then runs `cargo build` in the plugin's
/// workspace with `helpers` as its arguments, and passes the debug target
/// folder to the script as its one argument.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[non_exhaustive]
pub struct SmokeSection {
    /// The `sh` script for Linux and FreeBSD.
    #[serde(default)]
    pub unix: Option<String>,
    /// The PowerShell script for Windows.
    #[serde(default)]
    pub windows: Option<String>,
    /// The arguments of the `cargo build` that builds what the scripts need
    /// besides the program, such as a test master; none for nothing more.
    #[serde(default)]
    pub helpers: Vec<String>,
}

impl Manifest {
    /// Whether the manifest declares the service with this name.
    pub fn declares(&self, name: &str) -> bool {
        self.services.contains_key(name)
    }
}

#[cfg(feature = "manifest")]
pub use parse::ManifestError;

#[cfg(feature = "manifest")]
mod parse {
    use super::Manifest;
    use crate::services::service_info;
    use crate::version::SDK_MAJOR;

    /// The name of the service whose declaration `[secrets]` needs.
    const SECRETS_SERVICE: &str = "secrets";

    impl Manifest {
        /// Parses and checks the text of a `plugin.toml`, in this order:
        ///
        /// 1. a manifest that names another SDK major is refused as
        ///    [`SdkIncompatible`](ManifestError::SdkIncompatible) before its
        ///    other fields are read, because a manifest of another major may
        ///    have fields that this one does not know;
        /// 2. the manifest's shape, with every field known and the ID
        ///    checked by [`PluginId::parse`](crate::ids::PluginId::parse);
        /// 3. the names of its secrets, then a name in both lists of
        ///    `[secrets]`, then an entry or a legacy name that is a keyed
        ///    family or an entry of one;
        /// 4. its services, each one the SDK has, at the SDK's major;
        /// 5. that a manifest with secrets declares the service `secrets`;
        /// 6. that each grant under `[native]` gives its reason, which the
        ///    review of the manifest reads.
        pub fn parse(text: &str) -> Result<Self, ManifestError> {
            let table: toml::Table = text
                .parse()
                .map_err(|error| ManifestError::from_toml(&error))?;
            if let Some(sdk) = other_major(&table) {
                let id = table
                    .get("id")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned);
                return Err(ManifestError::SdkIncompatible { id, sdk });
            }
            let manifest: Self =
                toml::from_str(text).map_err(|error| ManifestError::from_toml(&error))?;
            manifest.check_secret_names()?;
            manifest.check_services()?;
            let has_entries = !manifest.secrets.entries.is_empty();
            let has_legacy = !manifest.secrets.legacy.is_empty();
            let has_keyed = !manifest.secrets.keyed.is_empty();
            let lists_secrets = has_entries || has_legacy || has_keyed;
            if lists_secrets && !manifest.declares(SECRETS_SERVICE) {
                return Err(ManifestError::SecretsWithoutService);
            }
            let without_reason = manifest
                .native
                .grants()
                .into_iter()
                .find(|(_, reason)| reason.trim().is_empty());
            if let Some((category, _)) = without_reason {
                return Err(ManifestError::NativeWithoutReason {
                    category: category.to_owned(),
                });
            }
            Ok(manifest)
        }

        fn check_secret_names(&self) -> Result<(), ManifestError> {
            let secrets = &self.secrets;
            let mut names = secrets
                .entries
                .iter()
                .chain(&secrets.legacy)
                .chain(&secrets.keyed);
            if let Some(name) = names.find(|name| !is_secret_name(name)) {
                return Err(ManifestError::SecretName { name: name.clone() });
            }
            let in_both = secrets
                .entries
                .iter()
                .find(|entry| secrets.legacy.contains(entry));
            if let Some(name) = in_both {
                return Err(ManifestError::SecretInBothLists { name: name.clone() });
            }
            // A fixed name inside a family would name the same entry as a
            // key of that family, under two readings of one name.
            let in_family = secrets.entries.iter().chain(&secrets.legacy).find(|name| {
                secrets
                    .keyed
                    .iter()
                    .any(|family| is_in_family(name, family))
            });
            if let Some(name) = in_family {
                return Err(ManifestError::SecretInFamily { name: name.clone() });
            }
            Ok(())
        }

        fn check_services(&self) -> Result<(), ManifestError> {
            for (name, &major) in &self.services {
                let Some(service) = service_info(name) else {
                    return Err(ManifestError::UnknownService { name: name.clone() });
                };
                if major != service.major {
                    return Err(ManifestError::ServiceMajor {
                        name: name.clone(),
                        major,
                    });
                }
            }
            Ok(())
        }
    }

    /// The SDK major that a manifest names, when it is a major other than
    /// this SDK's. A missing major, or one that is not a small whole number,
    /// is left to the strict parse, which refuses it.
    fn other_major(table: &toml::Table) -> Option<u16> {
        let sdk = table.get("sdk")?.as_integer()?;
        let sdk = u16::try_from(sdk).ok()?;
        (sdk != SDK_MAJOR).then_some(sdk)
    }

    /// Whether a text is words of ASCII letters, digits, `.`, `_` and `-`,
    /// joined by single slashes: the characters that the credential store
    /// accepts in a name, with no empty word, so a name never starts or ends
    /// with a slash. The store's limit on the length of a name is checked
    /// when the provider of `secrets` binds the names to the plugin.
    fn is_secret_name(name: &str) -> bool {
        name.split('/').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
    }

    /// Whether `name` is the keyed family `family` or a name below it.
    fn is_in_family(name: &str, family: &str) -> bool {
        let below = name
            .strip_prefix(family)
            .is_some_and(|rest| rest.starts_with('/'));
        name == family || below
    }

    /// Why a manifest was refused.
    ///
    /// The SDK adds reasons by addition, so the host treats a reason that it
    /// does not know as a refusal of the manifest.
    #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
    #[non_exhaustive]
    pub enum ManifestError {
        /// The text is not TOML, or does not have the manifest's shape: a
        /// field is missing, unknown or of the wrong type, or the ID breaks
        /// the rules of plugin IDs.
        #[error("the manifest is not valid: {message}")]
        Toml {
            /// What the parser reported, with the line and the column.
            message: String,
        },
        /// The manifest names another major of the plugin SDK.
        #[error(
            "the manifest{plugin} names major {sdk} of the plugin SDK, and this SDK is major {current}",
            plugin = of_plugin(.id.as_deref()),
            current = SDK_MAJOR
        )]
        SdkIncompatible {
            /// The plugin's ID as the manifest gives it, unchecked, when it
            /// is text.
            id: Option<String>,
            /// The major that the manifest names.
            sdk: u16,
        },
        /// The manifest declares a service that the SDK does not have.
        #[error("the manifest declares the service {name:?}, which the plugin SDK does not have")]
        UnknownService {
            /// The service's name.
            name: String,
        },
        /// The manifest declares a service at a major that the SDK does not
        /// have.
        #[error(
            "the manifest declares major {major} of the service {name}, which the plugin SDK does not have"
        )]
        ServiceMajor {
            /// The service's name.
            name: String,
            /// The major that the manifest names.
            major: u16,
        },
        /// A secret's name is not words of ASCII letters, digits, `.`, `_`
        /// and `-` joined by single slashes.
        #[error(
            "{name:?} is not a secret's name, which is words of ASCII letters, digits, \".\", \"_\" and \"-\" joined by single slashes"
        )]
        SecretName {
            /// The refused name.
            name: String,
        },
        /// A name is both an entry and a legacy name, so it would be stored
        /// under two names.
        #[error("the secret {name:?} is listed both under entries and under legacy")]
        SecretInBothLists {
            /// The name.
            name: String,
        },
        /// An entry or a legacy name is a keyed family, or a name below
        /// one, so it would name an entry that a key of the family names
        /// too.
        #[error("the secret {name:?} is a keyed family, or an entry of one")]
        SecretInFamily {
            /// The name.
            name: String,
        },
        /// The manifest lists secrets but does not declare the service
        /// `secrets`, which reaches them.
        #[error("the manifest lists secrets but does not declare the service secrets")]
        SecretsWithoutService,
        /// A grant under `[native]` has a reason that is empty or only white
        /// space.
        #[error("the manifest grants {category} under [native] without a reason")]
        NativeWithoutReason {
            /// The category, as `[native]` names it, such as `sockets`.
            category: String,
        },
    }

    impl ManifestError {
        fn from_toml(error: &toml::de::Error) -> Self {
            Self::Toml {
                message: error.to_string(),
            }
        }
    }

    /// The plugin that a refusal names, as a phrase that follows "the
    /// manifest".
    fn of_plugin(id: Option<&str>) -> String {
        id.map(|id| format!(" of {id:?}")).unwrap_or_default()
    }
}
