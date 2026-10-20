//! A plugin's manifest (ADR-0035, decision 1): what `Manifest::parse`
//! accepts and refuses, the JSON Schema that the SDK commits as
//! `plugin.schema.json`, which must be the manifest's own, so Tombi and
//! editors check a `plugin.toml` as the host does, and the rules that the SDK
//! commits as `plugin-rules.json`, which the build's generator checks
//! manifests with before any Rust compiles (decision 6).
//!
//! `MANNIE_UPDATE_EXPECTED=1` writes the schema from `Manifest` and the rules
//! from the SDK's constants instead of comparing; review the result before
//! committing it.

#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use mannie_plugin_sdk::ids::{
    CORE_TABLE_PREFIXES, IdError, PluginId, RESERVED_IDS, TAURI_PLUGIN_CRATE_PREFIX,
};
use mannie_plugin_sdk::manifest::{
    CratesSection, Manifest, ManifestError, NativeSection, SecretsSection, is_process_name,
};
use mannie_plugin_sdk::services::SERVICES;
use mannie_plugin_sdk::version::SDK_MAJOR;
use mannie_types::tools::{LOADER_NAME_PREFIXES, SECRET_NAME_PARTS};
use serde_json::{Value, json};

/// A manifest with the fields that every manifest has, and nothing else.
const MINIMAL: &str = r#"
id = "notes"
sdk = 1
order = 30

[rust]
entry = "mannie-notes"
"#;

/// The minimal manifest with more TOML after it.
fn with(more: &str) -> String {
    format!("{MINIMAL}\n{more}")
}

/// The message of a refusal of the manifest's shape.
fn toml_message(text: &str) -> String {
    match Manifest::parse(text) {
        Err(ManifestError::Toml { message }) => message,
        other => panic!("expected a refusal of the shape, got {other:?}"),
    }
}

#[test]
fn a_manifest_with_the_required_fields_parses_with_no_service_and_no_secret() {
    let manifest = Manifest::parse(MINIMAL).unwrap();

    assert_eq!(manifest.id, PluginId::parse("notes").unwrap());
    assert_eq!(manifest.sdk, SDK_MAJOR);
    assert_eq!(manifest.order, 30);
    assert_eq!(manifest.rust.entry, "mannie-notes");
    assert_eq!(manifest.services, BTreeMap::new());
    assert_eq!(manifest.secrets, SecretsSection::default());
    assert_eq!(manifest.crates, CratesSection::default());
    assert_eq!(manifest.processes, BTreeMap::new());
    assert_eq!(manifest.native, NativeSection::default());
    assert_eq!(manifest.native.grants(), Vec::<(&str, &str)>::new());
    assert!(!manifest.declares("executor-host"));
}

/// The minimal manifest with the service `processes`, running `program`
/// with a reason and the lines of `table` in its table.
fn running(program: &str, table: &str) -> String {
    let key = serde_json::to_string(program).unwrap();
    with(&format!(
        "[services]\nprocesses = 1\n\n[processes.{key}]\nreason = \"Runs it.\"\n{table}"
    ))
}

#[test]
fn a_manifest_lists_the_installed_programs_that_it_runs() {
    let manifest = Manifest::parse(&running(
        "git",
        "fixed = [\"-c\", \"core.fsmonitor=false\"]
inherit = [\"HOME\", \"SSH_AUTH_SOCK\"]

[processes.pandoc]
reason = \"Converts the notes.\"",
    ))
    .unwrap();
    let unknown = toml_message(&running("git", "shell = true"));
    let missing = toml_message(&with(
        "[services]\nprocesses = 1\n\n[processes.git]\nfixed = []",
    ));

    assert_eq!(
        manifest.processes.keys().collect::<Vec<_>>(),
        ["git", "pandoc"]
    );
    let git = &manifest.processes["git"];
    assert_eq!(git.reason, "Runs it.");
    assert_eq!(git.fixed, ["-c", "core.fsmonitor=false"]);
    assert_eq!(git.inherit, ["HOME", "SSH_AUTH_SOCK"]);
    let pandoc = &manifest.processes["pandoc"];
    assert_eq!(pandoc.reason, "Converts the notes.");
    assert!(pandoc.fixed.is_empty() && pandoc.inherit.is_empty());
    assert!(unknown.contains("unknown field `shell`"), "{unknown}");
    assert!(missing.contains("reason"), "{missing}");
}

#[test]
fn an_installed_program_without_a_reason_or_with_a_nul_in_a_fixed_argument_is_refused() {
    for reason in ["", " \\t"] {
        let text = with(&format!(
            "[services]\nprocesses = 1\n\n[processes.git]\nreason = \"{reason}\""
        ));

        assert_eq!(
            Manifest::parse(&text),
            Err(ManifestError::ProcessWithoutReason {
                program: "git".to_owned()
            }),
            "{reason:?}"
        );
    }
    let nul = running("git", "fixed = [\"-c\", \"a\\u0000b\"]");

    assert_eq!(
        Manifest::parse(&nul),
        Err(ManifestError::ProcessArgument {
            program: "git".to_owned()
        })
    );
}

#[test]
fn installed_programs_without_the_service_processes_are_refused_after_their_own_checks() {
    let without = with("[processes.git]\nreason = \"Runs it.\"");
    let bad_name_without = with("[processes.\"../git\"]\nreason = \"Runs it.\"");
    let service_alone = with("[services]\nprocesses = 1");

    assert_eq!(
        Manifest::parse(&without),
        Err(ManifestError::ProcessesWithoutService)
    );
    assert_eq!(
        Manifest::parse(&bad_name_without),
        Err(ManifestError::ProcessName {
            name: "../git".to_owned()
        })
    );
    assert_eq!(
        Manifest::parse(&service_alone).unwrap().processes,
        BTreeMap::new()
    );
}

/// The names over which `tests/scripts/plugins/manifests.test.mjs` runs the
/// generator's check of `[processes]`: `names` lists names of installed
/// programs and `inherited` names of the variables that they inherit, each
/// accepted or refused as `Manifest::parse` judges them.
fn processes_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugin-processes.json")
}

#[test]
fn every_name_of_the_processes_fixture_is_accepted_or_refused_as_it_is_listed() {
    let text = std::fs::read_to_string(processes_fixture()).unwrap();
    let fixture: Value = serde_json::from_str(&text).unwrap();
    let list = |group: &str, verdict: &str| -> Vec<String> {
        let names = fixture[group][verdict].as_array().unwrap();
        assert!(!names.is_empty(), "{group}.{verdict}");
        names
            .iter()
            .map(|name| name.as_str().unwrap().to_owned())
            .collect()
    };
    let inheriting = |name: &str| {
        let list = serde_json::to_string(&[name]).unwrap();
        running("git", &format!("inherit = {list}"))
    };

    for name in list("names", "accepted") {
        assert!(is_process_name(&name), "{name:?}");
        assert!(Manifest::parse(&running(&name, "")).is_ok(), "{name:?}");
    }
    for name in list("names", "refused") {
        assert!(!is_process_name(&name), "{name:?}");
        assert_eq!(
            Manifest::parse(&running(&name, "")),
            Err(ManifestError::ProcessName { name: name.clone() }),
            "{name:?}"
        );
    }
    for name in list("inherited", "accepted") {
        let manifest = Manifest::parse(&inheriting(&name)).unwrap();
        assert_eq!(
            manifest.processes["git"].inherit,
            std::slice::from_ref(&name),
            "{name:?}"
        );
    }
    for name in list("inherited", "refused") {
        assert_eq!(
            Manifest::parse(&inheriting(&name)),
            Err(ManifestError::ProcessInherits {
                program: "git".to_owned(),
                name: name.clone()
            }),
            "{name:?}"
        );
    }
}

#[test]
fn a_manifest_lists_its_programs() {
    let manifest = Manifest::parse(&with(
        r#"
[[programs]]
package = "mannie-notes-sync"
crates = ["mannie-notes-sync-link"]
headless = true
targets = ["x86_64-unknown-freebsd"]
signed = true
self-check = ["self-check", "--json"]
e2e = true
smoke = { unix = "ci/smoke.sh", helpers = ["--example", "smoke"] }

[[programs]]
package = "mannie-notes-tool"
"#,
    ))
    .unwrap();
    let unknown = toml_message(&with("[[programs]]\npackage = \"x\"\nsigning = true"));

    let [sync, tool] = manifest.programs.as_slice() else {
        panic!("expected two programs, got {:?}", manifest.programs);
    };
    assert_eq!(sync.package, "mannie-notes-sync");
    assert_eq!(sync.crates, ["mannie-notes-sync-link"]);
    assert_eq!(tool.crates, Vec::<String>::new());
    assert!(sync.headless && sync.signed && sync.e2e);
    assert_eq!(sync.targets, ["x86_64-unknown-freebsd"]);
    assert_eq!(sync.self_check, ["self-check", "--json"]);
    assert_eq!(sync.smoke.unix.as_deref(), Some("ci/smoke.sh"));
    assert_eq!(sync.smoke.windows, None);
    assert_eq!(sync.smoke.helpers, ["--example", "smoke"]);
    assert!(!tool.headless && !tool.signed && !tool.e2e);
    assert!(tool.targets.is_empty() && tool.self_check.is_empty());
    assert!(unknown.contains("signing"), "{unknown}");
}

#[test]
fn a_manifest_lists_the_crates_that_only_its_plugin_may_use() {
    let manifest = Manifest::parse(&with(
        "[crates]
exclusive = [\"tonic\", \"protox\"]",
    ))
    .unwrap();
    let unknown = toml_message(&with(
        "[crates]
shared = [\"tonic\"]",
    ));

    assert_eq!(manifest.crates.exclusive, ["tonic", "protox"]);
    assert!(unknown.contains("shared"), "{unknown}");
}

#[test]
fn a_manifest_grants_its_library_crates_categories_of_native_calls_with_their_reasons() {
    let manifest = Manifest::parse(&with(
        "[native]
sockets = \"The hub connects to workers.\"
file-writes = \"The notes are files.\"",
    ))
    .unwrap();
    let unknown = toml_message(&with(
        "[native]
threads = \"The sync runs on a thread.\"",
    ));

    assert_eq!(
        manifest.native.grants(),
        [
            ("file-writes", "The notes are files."),
            ("sockets", "The hub connects to workers."),
        ]
    );
    assert_eq!(manifest.native.processes, None);
    assert!(unknown.contains("unknown field `threads`"), "{unknown}");
}

#[test]
fn a_native_grant_without_a_reason_is_refused() {
    for (table, category) in [
        ("processes = \"\"", "processes"),
        (
            "sockets = \"Connects.\"\nfile-writes = \" \\t\"",
            "file-writes",
        ),
    ] {
        let text = with(&format!("[native]\n{table}"));

        assert_eq!(
            Manifest::parse(&text),
            Err(ManifestError::NativeWithoutReason {
                category: category.to_owned()
            }),
            "{table}"
        );
    }
}

#[test]
fn a_manifest_declares_every_service_of_the_sdk_at_its_major() {
    let services = SERVICES
        .iter()
        .map(|service| {
            format!(
                "# Used by the plugin.\n{} = {}\n",
                service.name, service.major
            )
        })
        .collect::<Vec<String>>()
        .concat();

    let manifest = Manifest::parse(&with(&format!("[services]\n{services}"))).unwrap();

    assert_eq!(manifest.services.len(), SERVICES.len());
    for service in SERVICES {
        assert!(manifest.declares(service.name), "{}", service.name);
        assert_eq!(manifest.services[service.name], service.major);
    }
    assert!(!manifest.declares("no-such-service"));
}

#[test]
fn an_unknown_field_is_refused_and_named() {
    // A key after `[rust]` belongs to `[rust]`, so the unknown key of the top
    // level comes before it.
    let at_the_top_level =
        "id = \"notes\"\nsdk = 1\norder = 30\ncolour = \"red\"\n\n[rust]\nentry = \"mannie-notes\"";
    let in_rust = "id = \"notes\"\nsdk = 1\norder = 30\n\n[rust]\nentry = \"mannie-notes\"\npath = \"crates\"";
    for (text, field) in [
        (at_the_top_level.to_owned(), "colour"),
        (with("[ui]\nentry = \"ui/index.ts\""), "ui"),
        (in_rust.to_owned(), "path"),
        (with("[secrets]\nentries = []\nshared = []"), "shared"),
    ] {
        let message = toml_message(&text);
        assert!(
            message.contains(&format!("unknown field `{field}`")),
            "{field}: {message}"
        );
    }
}

#[test]
fn a_missing_field_is_refused_and_named() {
    for (text, field) in [
        (
            "sdk = 1\norder = 30\n[rust]\nentry = \"mannie-notes\"",
            "id",
        ),
        (
            "id = \"notes\"\nsdk = 1\n[rust]\nentry = \"mannie-notes\"",
            "order",
        ),
        ("id = \"notes\"\nsdk = 1\norder = 30", "rust"),
        ("id = \"notes\"\nsdk = 1\norder = 30\n[rust]", "entry"),
    ] {
        let message = toml_message(text);
        assert!(message.contains(field), "{field}: {message}");
    }
}

#[test]
fn text_that_is_not_toml_is_refused() {
    let message = toml_message("id = \nsdk = 1");

    assert!(message.contains("line 1"), "{message}");
}

#[test]
fn an_unknown_service_is_refused() {
    for name in ["teleport", "executor_host", "Executor-Host", ""] {
        let text = with(&format!("[services]\n\"{name}\" = 1"));

        assert_eq!(
            Manifest::parse(&text),
            Err(ManifestError::UnknownService {
                name: name.to_owned()
            }),
            "{name}"
        );
    }
}

#[test]
fn another_major_of_a_service_is_refused() {
    for (name, major) in [("executor-host", 2), ("credentials", 0)] {
        let text = with(&format!("[services]\n{name} = {major}"));

        assert_eq!(
            Manifest::parse(&text),
            Err(ManifestError::ServiceMajor {
                name: name.to_owned(),
                major
            }),
            "{name}"
        );
    }
    let negative = toml_message(&with("[services]\ncredentials = -1"));
    assert!(negative.contains("credentials"), "{negative}");
}

#[test]
fn an_invalid_or_reserved_id_is_refused_with_the_rule_that_it_breaks() {
    for (id, rule) in [
        ("\"Notes\"", "lower-case words"),
        ("\"log_viewer\"", "lower-case words"),
        ("\"runtime\"", "reserved"),
        ("\"mannie\"", "reserved"),
        ("\"tauri-plugin-notes\"", "tauri-plugin-"),
        ("\"settings-sync\"", "settings_*"),
        ("7", "string"),
    ] {
        let text = format!("id = {id}\nsdk = 1\norder = 30\n\n[rust]\nentry = \"mannie-notes\"\n");

        let message = toml_message(&text);

        assert!(message.contains(rule), "{id}: {message}");
    }
}

#[test]
fn a_manifest_for_another_sdk_major_is_refused_before_its_other_fields_are_read() {
    // A manifest of SDK 2 may have tables and services that SDK 1 does not
    // know, and lack fields that SDK 1 needs.
    let future =
        "id = \"notes\"\nsdk = 2\n\n[ui]\nentry = \"ui/index.ts\"\n\n[services]\nteleport = 4\n";

    assert_eq!(
        Manifest::parse(future),
        Err(ManifestError::SdkIncompatible {
            id: Some("notes".to_owned()),
            sdk: 2
        })
    );
    for (text, id) in [
        ("sdk = 0", None),
        ("id = 7\nsdk = 0", None),
        ("id = \"Not An Id\"\nsdk = 0", Some("Not An Id")),
    ] {
        assert_eq!(
            Manifest::parse(text),
            Err(ManifestError::SdkIncompatible {
                id: id.map(str::to_owned),
                sdk: 0
            }),
            "{text}"
        );
    }
}

#[test]
fn a_missing_or_malformed_sdk_major_is_refused_as_the_shape() {
    for sdk in ["", "sdk = \"1\"", "sdk = 1.0", "sdk = 70000", "sdk = -1"] {
        let text =
            format!("id = \"notes\"\n{sdk}\norder = 30\n\n[rust]\nentry = \"mannie-notes\"\n");

        let message = toml_message(&text);

        assert!(message.contains("sdk"), "{sdk}: {message}");
    }
}

#[test]
fn a_secret_name_in_both_lists_is_refused() {
    let text = with("[secrets]\nentries = [\"token\", \"link/key\"]\nlegacy = [\"link/key\"]");

    assert_eq!(
        Manifest::parse(&text),
        Err(ManifestError::SecretInBothLists {
            name: "link/key".to_owned()
        })
    );
    // A name twice in one list names one entry, so it reaches the next check,
    // which needs the service.
    for list in ["entries", "legacy"] {
        let twice = with(&format!("[secrets]\n{list} = [\"token\", \"token\"]"));

        assert_eq!(
            Manifest::parse(&twice),
            Err(ManifestError::SecretsWithoutService),
            "{list}"
        );
    }
}

#[test]
fn secrets_without_the_service_secrets_are_refused() {
    for secrets in [
        "entries = [\"token\"]",
        "legacy = [\"link/master-key\"]",
        "entries = [\"token\"]\nlegacy = [\"link/master-key\"]",
        "keyed = [\"host\"]",
    ] {
        for services in ["", "[services]\nexecutor-host = 1\ncredentials = 1"] {
            let text = with(&format!("{services}\n[secrets]\n{secrets}"));

            assert_eq!(
                Manifest::parse(&text),
                Err(ManifestError::SecretsWithoutService),
                "{services} {secrets}"
            );
        }
    }
    let empty = Manifest::parse(&with("[secrets]\nentries = []\nlegacy = []\nkeyed = []")).unwrap();
    assert_eq!(empty.secrets, SecretsSection::default());
}

#[test]
fn an_entry_or_a_legacy_name_that_is_a_keyed_family_or_inside_one_is_refused() {
    for (list, name) in [
        ("entries", "host"),
        ("entries", "host/main"),
        ("legacy", "host/main/key"),
    ] {
        let text = with(&format!(
            "[services]\nsecrets = 1\n\n[secrets]\nkeyed = [\"host\"]\n{list} = [\"{name}\"]"
        ));

        assert_eq!(
            Manifest::parse(&text),
            Err(ManifestError::SecretInFamily {
                name: name.to_owned()
            }),
            "{list}: {name}"
        );
    }
    // A name that only starts with the family's letters is not inside it.
    let beside =
        with("[services]\nsecrets = 1\n\n[secrets]\nkeyed = [\"host\"]\nentries = [\"hostname\"]");
    let entries = Manifest::parse(&beside).unwrap().secrets.entries;
    assert_eq!(entries, ["hostname"]);
}

#[test]
fn an_entry_of_a_keyed_family_is_kept_below_the_plugins_id_with_a_key_of_letters_digits_and_hyphens()
 {
    let text = with(
        "[services]\nsecrets = 1\n\n[secrets]\nentries = [\"token\"]\nlegacy = [\"link/master-key\"]\nkeyed = [\"host\", \"sync/peer\"]",
    );
    let secrets = Manifest::parse(&text).unwrap().secrets;
    let longest = "k".repeat(64);
    let longest_entry = format!("host/{longest}");
    let longest_stored = format!("notes/host/{longest}");

    for (entry, stored) in [
        ("token", "notes/token"),
        ("link/master-key", "link/master-key"),
        (
            "host/0199e0f2-0000-7000-8000-0000000000aa",
            "notes/host/0199e0f2-0000-7000-8000-0000000000aa",
        ),
        ("sync/peer/Office-2", "notes/sync/peer/Office-2"),
        (longest_entry.as_str(), longest_stored.as_str()),
    ] {
        assert_eq!(
            secrets.stored_name("notes", entry).as_deref(),
            Some(stored),
            "{entry}"
        );
    }
    let too_long = format!("host/{longest}k");
    for entry in [
        "host",
        "host/",
        "host/a_b",
        "host/a.b",
        "host/a b",
        "host/a/b",
        "sync/x",
        "notes/token",
        too_long.as_str(),
    ] {
        assert_eq!(secrets.stored_name("notes", entry), None, "{entry}");
    }
}

#[test]
fn a_secret_name_is_words_of_letters_digits_dots_underscores_and_hyphens_joined_by_slashes() {
    for name in [
        "",
        "/key",
        "key/",
        "link//key",
        "api key",
        "kéy",
        "link|key",
        "key:1",
    ] {
        for list in ["entries", "legacy", "keyed"] {
            let text = with(&format!("[secrets]\n{list} = [\"{name}\"]"));

            assert_eq!(
                Manifest::parse(&text),
                Err(ManifestError::SecretName {
                    name: name.to_owned()
                }),
                "{list}: {name}"
            );
        }
    }
    // Names that pass reach the next check, which needs the service.
    for name in ["link/master-key", "api.v2_token-1", "A/b/C"] {
        let text = with(&format!("[secrets]\nentries = [\"{name}\"]"));

        assert_eq!(
            Manifest::parse(&text),
            Err(ManifestError::SecretsWithoutService),
            "{name}"
        );
    }
}

#[test]
fn a_secret_name_is_checked_before_the_services() {
    let text = with("[services]\nteleport = 1\n\n[secrets]\nentries = [\"\"]");

    assert_eq!(
        Manifest::parse(&text),
        Err(ManifestError::SecretName {
            name: String::new()
        })
    );
}

#[test]
fn a_refusal_says_what_was_refused() {
    for (error, text) in [
        (
            ManifestError::Toml {
                message: "missing field `id`".to_owned(),
            },
            "the manifest is not valid: missing field `id`",
        ),
        (
            ManifestError::SdkIncompatible {
                id: Some("notes".to_owned()),
                sdk: 2,
            },
            "the manifest of \"notes\" names major 2 of the plugin SDK, and this SDK is major 1",
        ),
        (
            ManifestError::SdkIncompatible { id: None, sdk: 2 },
            "the manifest names major 2 of the plugin SDK, and this SDK is major 1",
        ),
        (
            ManifestError::UnknownService {
                name: "teleport".to_owned(),
            },
            "the manifest declares the service \"teleport\", which the plugin SDK does not have",
        ),
        (
            ManifestError::ServiceMajor {
                name: "executor-host".to_owned(),
                major: 2,
            },
            "the manifest declares major 2 of the service executor-host, which the plugin SDK does not have",
        ),
        (
            ManifestError::SecretName {
                name: "api key".to_owned(),
            },
            "\"api key\" is not a secret's name, which is words of ASCII letters, digits, \".\", \"_\" and \"-\" joined by single slashes",
        ),
        (
            ManifestError::SecretInBothLists {
                name: "token".to_owned(),
            },
            "the secret \"token\" is listed both under entries and under legacy",
        ),
        (
            ManifestError::SecretInFamily {
                name: "host/main".to_owned(),
            },
            "the secret \"host/main\" is a keyed family, or an entry of one",
        ),
        (
            ManifestError::SecretsWithoutService,
            "the manifest lists secrets but does not declare the service secrets",
        ),
        (
            ManifestError::NativeWithoutReason {
                category: "sockets".to_owned(),
            },
            "the manifest grants sockets under [native] without a reason",
        ),
        (
            ManifestError::ProcessName {
                name: "../git".to_owned(),
            },
            "\"../git\" is not the name of an installed program, which is a letter or a digit, then letters, digits, \".\", \"_\" and \"-\", at most 64 characters, never ending in a dot",
        ),
        (
            ManifestError::ProcessWithoutReason {
                program: "git".to_owned(),
            },
            "the manifest runs the program git under [processes] without a reason",
        ),
        (
            ManifestError::ProcessArgument {
                program: "git".to_owned(),
            },
            "a fixed argument of the program git holds a NUL character",
        ),
        (
            ManifestError::ProcessInherits {
                program: "git".to_owned(),
                name: "GITHUB_TOKEN".to_owned(),
            },
            "the program git inherits \"GITHUB_TOKEN\", which is not a variable's name, looks like a secret's or loads libraries",
        ),
        (
            ManifestError::ProcessesWithoutService,
            "the manifest lists programs under [processes] but does not declare the service processes",
        ),
    ] {
        assert_eq!(error.to_string(), text);
    }
}

fn committed_schema() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugin.schema.json")
}

fn manifest_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(Manifest)).unwrap()
}

#[test]
fn the_committed_schema_is_the_manifests_schema() {
    let actual = manifest_schema();
    if std::env::var_os("MANNIE_UPDATE_EXPECTED").is_some() {
        let text = serde_json::to_string_pretty(&actual).unwrap();
        std::fs::write(committed_schema(), format!("{text}\n")).unwrap();
        return;
    }
    let text = std::fs::read_to_string(committed_schema()).unwrap();
    let committed: Value = serde_json::from_str(&text).unwrap();

    assert_eq!(
        actual,
        committed,
        "the manifest's schema changed; review it and update {}",
        committed_schema().display()
    );
}

/// The IDs over which `tests/scripts/plugin-schema.test.mjs` runs the
/// schema's pattern of an ID as a JavaScript regular expression, the dialect
/// of JSON Schema's patterns: `follows` lists texts with the syntax of plugin
/// IDs, some of them refused by later rules, and `breaks` lists texts without
/// it.
fn id_syntax_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugin-id-syntax.json")
}

#[test]
fn every_id_of_the_syntax_fixture_follows_or_breaks_the_syntax_as_it_is_listed() {
    let text = std::fs::read_to_string(id_syntax_fixture()).unwrap();
    let fixture: Value = serde_json::from_str(&text).unwrap();

    for (list, has_syntax) in [("follows", true), ("breaks", false)] {
        let ids = fixture[list].as_array().unwrap();
        assert!(!ids.is_empty(), "{list}");
        for id in ids {
            let id = id.as_str().unwrap();
            let breaks_syntax = matches!(PluginId::parse(id), Err(IdError::Syntax { .. }));
            assert_eq!(!breaks_syntax, has_syntax, "{list}: {id:?}");
        }
    }
}

fn committed_rules() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugin-rules.json")
}

/// The rules that `scripts/plugins/generate.mjs` checks manifests with, from
/// the constants that `Manifest::parse` and `PluginId::parse` use: the SDK's
/// major, the reserved IDs, the start of the names of Tauri's plugin crates,
/// the table prefixes that are not a plugin's, each service with its major,
/// and the parts and starts of the names of variables that an installed
/// program may not inherit. The pattern of an ID's syntax is the schema's.
/// The services that reach sensitive data are listed too, for `task
/// plugins:grants`.
fn manifest_rules() -> Value {
    let services: BTreeMap<&str, u16> = SERVICES
        .iter()
        .map(|service| (service.name, service.major))
        .collect();
    let sensitive: Vec<&str> = SERVICES
        .iter()
        .filter(|service| service.sensitive)
        .map(|service| service.name)
        .collect();
    json!({
        "coreTablePrefixes": CORE_TABLE_PREFIXES,
        "loaderNamePrefixes": LOADER_NAME_PREFIXES,
        "reservedIds": RESERVED_IDS,
        "sdkMajor": SDK_MAJOR,
        "secretNameParts": SECRET_NAME_PARTS,
        "sensitiveServices": sensitive,
        "services": services,
        "tauriPluginCratePrefix": TAURI_PLUGIN_CRATE_PREFIX,
    })
}

#[test]
fn the_committed_rules_are_the_sdks_rules() {
    let actual = manifest_rules();
    if std::env::var_os("MANNIE_UPDATE_EXPECTED").is_some() {
        let text = serde_json::to_string_pretty(&actual).unwrap();
        std::fs::write(committed_rules(), format!("{text}\n")).unwrap();
        return;
    }
    let text = std::fs::read_to_string(committed_rules()).unwrap();
    let committed: Value = serde_json::from_str(&text).unwrap();

    assert_eq!(
        actual,
        committed,
        "the SDK's rules of manifests changed; review them and update {}",
        committed_rules().display()
    );
}

/// The IDs over which `tests/scripts/plugins/manifests.test.mjs` runs the
/// generator's check of an ID: `accepted` lists IDs that `PluginId::parse`
/// accepts, and each list of `refused` the IDs that it refuses for one rule,
/// named as the generator names it.
fn id_rules_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugin-ids.json")
}

/// The generator's name of the rule that an ID breaks.
fn rule_name(error: &IdError) -> &'static str {
    match error {
        IdError::Syntax { .. } => "syntax",
        IdError::Reserved { .. } => "reserved",
        IdError::TauriPrefix { .. } => "tauri_prefix",
        IdError::NestsCorePrefix { .. } => "nests_core_prefix",
        _ => "unknown",
    }
}

#[test]
fn every_id_of_the_rules_fixture_is_accepted_or_refused_for_the_rule_it_is_listed_under() {
    let text = std::fs::read_to_string(id_rules_fixture()).unwrap();
    let fixture: Value = serde_json::from_str(&text).unwrap();

    let accepted = fixture["accepted"].as_array().unwrap();
    assert_ne!(accepted, &Vec::<Value>::new());
    for id in accepted {
        let id = id.as_str().unwrap();
        assert_eq!(
            PluginId::parse(id).map(|id| id.to_string()),
            Ok(id.to_owned())
        );
    }
    let refused = fixture["refused"].as_object().unwrap();
    assert_eq!(
        refused.keys().map(String::as_str).collect::<Vec<_>>(),
        ["nests_core_prefix", "reserved", "syntax", "tauri_prefix"]
    );
    for (rule, ids) in refused {
        let ids = ids.as_array().unwrap();
        assert!(!ids.is_empty(), "{rule}");
        for id in ids {
            let id = id.as_str().unwrap();
            let error = PluginId::parse(id).unwrap_err();
            assert_eq!(rule_name(&error), rule, "{id:?}");
        }
    }
}

#[test]
fn the_schema_refuses_unknown_fields_in_every_table() {
    let schema = manifest_schema();

    assert_eq!(schema["additionalProperties"], false);
    let definitions = schema["$defs"].as_object().unwrap();
    assert!(!definitions.is_empty());
    for (name, definition) in definitions {
        assert_eq!(definition["additionalProperties"], false, "{name}");
    }
}
