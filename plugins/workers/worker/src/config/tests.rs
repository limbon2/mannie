use std::path::{Path, PathBuf};

use mannie_types::ceiling::CeilingMode;
use mannie_types::clock::UtcMillis;
use tempfile::TempDir;

use super::{ConfigError, initial_file, parse};
use crate::paths::Folders;

/// A configuration folder with an allowed folder and a project folder.
struct Host {
    _root: TempDir,
    folders: Folders,
    work: PathBuf,
}

impl Host {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir_all(work.join("app")).unwrap();
        let folders = Folders {
            config: root.path().join("etc"),
            data: root.path().join("data"),
            program: root.path().join("program"),
        };
        Self {
            _root: root,
            folders,
            work,
        }
    }

    fn file(&self) -> PathBuf {
        self.folders.config.join("worker.toml")
    }

    fn parse(&self, text: &str) -> Result<super::Config, ConfigError> {
        parse(text, &self.file(), &self.folders, UtcMillis::EPOCH)
    }

    /// A path as a TOML literal string.
    fn literal(path: &Path) -> String {
        format!("'{}'", path.display())
    }
}

fn invalid_key(error: ConfigError) -> (String, String) {
    match error {
        ConfigError::Invalid { key, problem, .. } => (key, problem),
        other => panic!("expected an invalid value, got {other}"),
    }
}

fn syntax_message(error: ConfigError) -> String {
    match error {
        ConfigError::Syntax { message, .. } => message,
        other => panic!("expected a syntax error, got {other}"),
    }
}

#[test]
fn a_configuration_with_every_key_is_read() {
    let host = Host::new();
    let app = host.work.join("app");
    let text = format!(
        "version = 1\n\
         name = \"build-01\"\n\
         purpose = \"Builds and tests the backend services\"\n\
         listen = \"127.0.0.1:7500\"\n\
         data_dir = {data}\n\
         host_instructions = {instructions}\n\
         max_parallel_tasks = 2\n\
         [ceiling]\n\
         mode = \"autonomous\"\n\
         allowed_folders = [{work}]\n\
         rules = [{{ command = \"git status\" }}, {{ command = \"cargo test\", folder = {app} }}]\n\
         allow_full_access_from_master = false\n",
        data = Host::literal(&host.work.join("data")),
        instructions = Host::literal(&host.work.join("host.md")),
        work = Host::literal(&host.work),
        app = Host::literal(&app),
    );

    let config = host.parse(&text).unwrap();

    assert_eq!(config.name, "build-01");
    assert_eq!(config.purpose, "Builds and tests the backend services");
    assert_eq!(config.listen.to_string(), "127.0.0.1:7500");
    assert_eq!(config.data_dir, host.work.join("data"));
    assert_eq!(config.host_instructions, host.work.join("host.md"));
    assert_eq!(config.max_parallel_tasks, 2);
    assert_eq!(config.ceiling.mode, CeilingMode::Autonomous);
    assert_eq!(
        config.ceiling.allowed_folders,
        [PathBuf::from(
            mannie_tools::canonical_folder(&host.work).unwrap()
        )]
    );
    let words: Vec<_> = config
        .ceiling
        .rules
        .iter()
        .map(|rule| rule.words.join(" "))
        .collect();
    assert_eq!(words, ["git status", "cargo test"]);
    assert_eq!(config.ceiling.rules[0].folder, None);
    assert_eq!(
        config.ceiling.rules[1].folder,
        Some(mannie_tools::canonical_folder(&app).unwrap())
    );
    assert!(!config.accepts_master_full_access);
}

#[test]
fn a_host_in_full_access_is_read() {
    let host = Host::new();

    let config = host
        .parse("version = 1\nname = \"build-01\"\n[ceiling]\nmode = \"full_access\"\n")
        .unwrap();

    assert_eq!(config.ceiling.mode, CeilingMode::FullAccess);
    assert!(config.accepts_master_full_access);
}

#[test]
fn the_keys_left_out_take_their_defaults() {
    let host = Host::new();

    let config = host.parse("version = 1\nname = \"build-01\"\n").unwrap();

    assert_eq!(config.purpose, "");
    assert_eq!(config.listen.to_string(), "0.0.0.0:7445");
    assert_eq!(config.data_dir, host.folders.data);
    assert_eq!(
        config.host_instructions,
        host.folders.config.join("host-instructions.md")
    );
    assert_eq!(config.max_parallel_tasks, 4);
    assert_eq!(config.ceiling.mode, CeilingMode::Ask);
    assert_eq!(config.ceiling.allowed_folders.len(), 0);
    assert_eq!(config.ceiling.rules.len(), 0);
    assert!(config.accepts_master_full_access);
}

#[test]
fn a_newer_version_is_refused_for_its_version_before_its_keys() {
    let host = Host::new();

    let error = host
        .parse("version = 2\nname = \"build-01\"\nnew_key = true\n")
        .unwrap_err();

    assert!(
        matches!(error, ConfigError::NewerVersion { version: 2, .. }),
        "{error}"
    );
    assert!(error.to_string().contains("version 2"), "{error}");
}

#[test]
fn a_missing_or_old_version_is_refused() {
    let host = Host::new();

    let missing = invalid_key(host.parse("name = \"build-01\"\n").unwrap_err());
    let old = invalid_key(
        host.parse("version = 0\nname = \"build-01\"\n")
            .unwrap_err(),
    );
    let text = invalid_key(
        host.parse("version = \"1\"\nname = \"build-01\"\n")
            .unwrap_err(),
    );

    assert_eq!(missing.0, "version");
    assert_eq!(old.0, "version");
    assert_eq!(text.0, "version");
}

#[test]
fn an_unknown_key_is_refused_by_its_name() {
    let host = Host::new();

    let top = syntax_message(
        host.parse("version = 1\nname = \"build-01\"\nport = 7445\n")
            .unwrap_err(),
    );
    let nested = syntax_message(
        host.parse("version = 1\nname = \"build-01\"\n[ceiling]\nmode = \"ask\"\nfolders = []\n")
            .unwrap_err(),
    );

    assert!(top.contains("port"), "{top}");
    assert!(nested.contains("folders"), "{nested}");
}

#[test]
fn a_missing_name_and_a_bad_mode_are_refused() {
    let host = Host::new();

    let missing = syntax_message(host.parse("version = 1\n").unwrap_err());
    let mode = syntax_message(
        host.parse("version = 1\nname = \"build-01\"\n[ceiling]\nmode = \"anything\"\n")
            .unwrap_err(),
    );

    assert!(missing.contains("name"), "{missing}");
    assert!(mode.contains("anything"), "{mode}");
}

#[test]
fn values_out_of_their_range_are_refused_with_their_key() {
    let host = Host::new();
    let long_name = "x".repeat(65);
    let cases = [
        (format!("name = \"{long_name}\""), "name"),
        ("name = \"   \"".to_owned(), "name"),
        ("name = \"a\\u0007b\"".to_owned(), "name"),
        (
            format!("name = \"w\"\npurpose = \"{}\"", "p".repeat(1_001)),
            "purpose",
        ),
        ("name = \"w\"\nlisten = \"7445\"".to_owned(), "listen"),
        (
            "name = \"w\"\nmax_parallel_tasks = 0".to_owned(),
            "max_parallel_tasks",
        ),
        (
            "name = \"w\"\nmax_parallel_tasks = 17".to_owned(),
            "max_parallel_tasks",
        ),
        ("name = \"w\"\ndata_dir = \"data\"".to_owned(), "data_dir"),
        (
            "name = \"w\"\nhost_instructions = \"host.md\"".to_owned(),
            "host_instructions",
        ),
    ];

    for (body, key) in cases {
        let error = host.parse(&format!("version = 1\n{body}\n")).unwrap_err();
        assert_eq!(invalid_key(error).0, key, "{body}");
    }
}

#[test]
fn ceiling_folders_must_exist_and_rules_must_be_whole_words() {
    let host = Host::new();
    let missing = host.work.join("missing");
    let cases = [
        (
            format!("allowed_folders = [{}]", Host::literal(&missing)),
            "ceiling.allowed_folders[0]",
        ),
        (
            "allowed_folders = ['relative']".to_owned(),
            "ceiling.allowed_folders[0]",
        ),
        (
            "rules = [{ command = \"git status\" }, { command = \"make; rm -rf /\" }]".to_owned(),
            "ceiling.rules[1].command",
        ),
        (
            "rules = [{ command = \"   \" }]".to_owned(),
            "ceiling.rules[0].command",
        ),
        (
            format!(
                "rules = [{{ command = \"cargo test\", folder = {} }}]",
                Host::literal(&missing)
            ),
            "ceiling.rules[0].folder",
        ),
    ];

    for (body, key) in cases {
        let error = host
            .parse(&format!("version = 1\nname = \"w\"\n[ceiling]\n{body}\n"))
            .unwrap_err();
        let (found, problem) = invalid_key(error);
        assert_eq!(found, key, "{body}: {problem}");
    }
}

#[test]
fn the_initial_file_is_a_valid_configuration_with_the_hosts_name() {
    let host = Host::new();

    let config = host
        .parse(&initial_file("build-01", &host.folders))
        .unwrap();
    let odd = host
        .parse(&initial_file("name \"with\" quotes", &host.folders))
        .unwrap();

    assert_eq!(config.name, "build-01");
    assert_eq!(config.ceiling.mode, CeilingMode::Ask);
    assert_eq!(config.data_dir, host.folders.data);
    assert_eq!(
        odd.name, "mannie-worker",
        "a name that needs quoting is replaced"
    );
}
