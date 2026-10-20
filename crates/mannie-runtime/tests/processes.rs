//! The service `processes` (ADR-0038, decisions 7 and 10): the runtime runs
//! only the installed programs that a plugin's manifest declares, with the
//! manifest's fixed arguments first and the plugin's own exactly as given,
//! and with the plugin's variables. It refuses a run past the limits, in a
//! working directory that is not an absolute folder or lies in Mannie's
//! folders, or with a variable that looks like a secret, loads libraries or
//! replaces the environment of commands. A run for a mention or a command
//! reads only where a model's reads in its place run without asking.
//!
//! The tests run the installed git, which the plugin Git needs.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::host::PluginBinding;
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::services::processes::{
    ProcessError, Processes, ProcessesApi, ProgramEnding, ProgramRun, ReadPlace,
};
use mannie_runtime::testing::{Harness, HarnessOptions, reply};
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::tools::SetFullAccess;
use serde_json::json;
use tempfile::TempDir;

/// The plugin's ID.
const PLUGIN: &str = "history";

/// A runtime with a project and a folder outside it, and the service bound
/// to a plugin whose manifest declares git with a fixed setting.
struct Setup {
    harness: Harness,
    _workspace: TempDir,
    _outside: TempDir,
    root: PathBuf,
    outside: PathBuf,
    project: ProjectId,
    processes: Arc<dyn ProcessesApi>,
}

impl Setup {
    async fn new() -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .await;
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let project = harness
            .runtime
            .add_project(workspace.path().to_str().unwrap().to_owned())
            .await
            .unwrap();
        let outside_path = PathBuf::from(mannie_tools::canonical_folder(outside.path()).unwrap());
        let processes = bind(&harness, &manifest());
        Self {
            harness,
            _workspace: workspace,
            _outside: outside,
            root: PathBuf::from(&project.folder),
            outside: outside_path,
            project: project.id,
            processes,
        }
    }

    /// A conversation of the project, whose first reply ended.
    async fn conversation(&self) -> ConversationId {
        self.harness.model.push_reply(reply("Done."));
        let started = self
            .harness
            .runtime
            .start_conversation(
                StartConversation::new("OWNER: hello").with_project(Some(self.project.clone())),
            )
            .await
            .unwrap();
        self.harness
            .wait_for_run(&started.conversation_id, &started.run_id)
            .await;
        started.conversation_id
    }

    /// A run of git in the project's folder.
    fn git(&self, arguments: &[&str]) -> ProgramRun {
        git_in(&self.root, arguments)
    }
}

/// A run of git in `folder`.
fn git_in(folder: &Path, arguments: &[&str]) -> ProgramRun {
    let arguments = arguments
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect();
    ProgramRun::new("git", folder, arguments)
}

/// The manifest of the plugin, which runs git with a fixed author's name.
fn manifest() -> Manifest {
    serde_json::from_value(json!({
        "id": PLUGIN,
        "sdk": 1,
        "order": 40,
        "rust": { "entry": "mannie-history" },
        "services": { "processes": 1 },
        "processes": {
            "git": {
                "reason": "Reads the history.",
                "fixed": ["-c", "user.name=Fixed Name"],
                "inherit": ["HOME"],
            },
        },
    }))
    .unwrap()
}

/// The runtime's service `processes`, bound to the plugin of `manifest`.
fn bind(harness: &Harness, manifest: &Manifest) -> Arc<dyn ProcessesApi> {
    let id = PluginId::parse(PLUGIN).unwrap();
    let binding = PluginBinding::new(&id, manifest, HostProfile::Development);
    let provider = harness
        .runtime
        .service_providers()
        .into_iter()
        .find(|provider| provider.name() == "processes")
        .expect("the runtime provides the service processes");
    provider
        .bind(&binding)
        .unwrap()
        .get::<Processes>()
        .expect("the bound service is processes")
}

#[tokio::test]
async fn a_run_passes_the_fixed_arguments_first_and_its_own_exactly() {
    let setup = Setup::new().await;
    let tricky = [
        "a b",
        "c'd",
        "\"quoted\"",
        "back\\slash\\",
        "",
        "--end",
        "\u{fc}n\u{ef}",
    ];
    let mut arguments = vec!["rev-parse", "--sq-quote"];
    arguments.extend(tricky);

    let quoted = setup.processes.run(setup.git(&arguments)).await.unwrap();
    let name = setup
        .processes
        .run(setup.git(&["config", "user.name"]))
        .await
        .unwrap();

    assert!(quoted.succeeded(), "{quoted:?}");
    assert_eq!(
        String::from_utf8(quoted.output).unwrap(),
        " 'a b' 'c'\\''d' '\"quoted\"' 'back\\slash\\' '' '--end' '\u{fc}n\u{ef}'\n"
    );
    assert_eq!(name.output, b"Fixed Name\n");
}

#[tokio::test]
async fn input_variables_and_output_travel_as_bytes() {
    let setup = Setup::new().await;

    let hashed = setup
        .processes
        .run(
            setup
                .git(&["hash-object", "--stdin"])
                .with_input(b"hello\n".to_vec()),
        )
        .await
        .unwrap();
    let ident = setup
        .processes
        .run(
            setup
                .git(&["var", "GIT_AUTHOR_IDENT"])
                .with_variable("GIT_AUTHOR_NAME", "Set Name")
                .with_variable("GIT_AUTHOR_EMAIL", "set@example.com")
                .with_variable("GIT_AUTHOR_DATE", "@1700000000 +0000"),
        )
        .await
        .unwrap();
    let cut = setup
        .processes
        .run(
            setup
                .git(&["rev-parse", "--sq-quote", &"x".repeat(4096)])
                .with_output_limit(16),
        )
        .await
        .unwrap();

    assert_eq!(hashed.output, b"ce013625030ba8dba906f756967f9e9ca394464a\n");
    assert_eq!(hashed.errors, b"");
    assert_eq!(
        ident.output,
        b"Set Name <set@example.com> 1700000000 +0000\n"
    );
    assert_eq!(cut.ending, ProgramEnding::OutputFull);
    assert!(cut.output.len() <= 16, "{}", cut.output.len());
    assert!(cut.output_cut(), "{cut:?}");
}

#[tokio::test]
async fn only_the_programs_of_the_manifest_are_found_and_run() {
    let setup = Setup::new().await;
    let undeclared = || ProcessError::NotDeclared {
        program: "cargo".to_owned(),
    };

    let located = setup.processes.locate("git").await.unwrap();
    let located_again = setup.processes.locate("git").await.unwrap();
    let other_located = setup.processes.locate("cargo").await;
    let other_run = setup
        .processes
        .run(ProgramRun::new(
            "cargo",
            &setup.root,
            vec!["--version".to_owned()],
        ))
        .await;

    assert!(located.is_absolute(), "{located:?}");
    assert_eq!(
        located.file_stem().and_then(|stem| stem.to_str()),
        Some("git")
    );
    assert_eq!(located_again, located);
    assert_eq!(other_located, Err(undeclared()));
    assert_eq!(other_run, Err(undeclared()));
}

#[tokio::test]
async fn a_run_past_a_limit_or_outside_an_absolute_folder_is_refused_before_it_starts() {
    let setup = Setup::new().await;
    let version = || setup.git(&["--version"]);
    let cases = [
        ("no time", version().with_timeout_seconds(0)),
        ("too long", version().with_timeout_seconds(601)),
        ("no output", version().with_output_limit(0)),
        (
            "too much output",
            version().with_output_limit(16 * 1024 * 1024 + 1),
        ),
        (
            "too much input",
            version().with_input(vec![b'x'; 16 * 1024 * 1024 + 1]),
        ),
        (
            "a relative folder",
            git_in(Path::new("work"), &["--version"]),
        ),
        (
            "a missing folder",
            git_in(&setup.root.join("missing"), &["--version"]),
        ),
        (
            "Mannie's data folder",
            git_in(setup.harness.data_folder(), &["--version"]),
        ),
    ];

    for (case, run) in cases {
        let refused = setup.processes.run(run).await;

        assert!(
            matches!(refused, Err(ProcessError::InvalidRun { .. })),
            "{case}: {refused:?}"
        );
    }
    let longest = setup
        .processes
        .run(version().with_timeout_seconds(600))
        .await
        .unwrap();
    assert!(longest.succeeded(), "{longest:?}");
}

#[tokio::test]
async fn a_variable_that_looks_secret_loads_libraries_or_replaces_the_commands_environment_is_refused()
 {
    let setup = Setup::new().await;

    for name in [
        "GITHUB_TOKEN",
        "LD_PRELOAD",
        "DYLD_INSERT_LIBRARIES",
        "PATH",
        "1BAD",
    ] {
        let refused = setup
            .processes
            .run(setup.git(&["--version"]).with_variable(name, "x"))
            .await;

        assert_eq!(
            refused,
            Err(ProcessError::VariableRefused {
                name: name.to_owned()
            }),
            "{name}"
        );
    }
}

#[tokio::test]
async fn a_mentions_run_reads_where_its_conversation_reads_without_asking() {
    let setup = Setup::new().await;
    let conversation = setup.conversation().await;
    let reading = |folders: Vec<PathBuf>| {
        setup
            .git(&["--version"])
            .checked_as(ReadPlace::Conversation {
                conversation: conversation.clone(),
                folders,
            })
    };

    let inside = setup.processes.run(reading(vec![setup.root.clone()])).await;
    let outside = setup
        .processes
        .run(reading(vec![setup.root.clone(), setup.outside.clone()]))
        .await;
    let protected =
        PathBuf::from(mannie_tools::canonical_folder(setup.harness.data_folder()).unwrap());
    let in_mannie = setup.processes.run(reading(vec![protected.clone()])).await;
    setup
        .harness
        .runtime
        .set_full_access(SetFullAccess {
            conversation_id: conversation.clone(),
            enabled: true,
        })
        .await
        .unwrap();
    let outside_in_full_access = setup
        .processes
        .run(reading(vec![setup.outside.clone()]))
        .await;

    assert!(inside.unwrap().succeeded());
    assert_eq!(
        outside,
        Err(ProcessError::ReadNotAllowed {
            path: setup.outside.display().to_string()
        })
    );
    assert_eq!(
        in_mannie,
        Err(ProcessError::ReadNotAllowed {
            path: protected.display().to_string()
        })
    );
    assert!(outside_in_full_access.unwrap().succeeded());
}

#[tokio::test]
async fn a_commands_run_reads_where_the_projects_conversations_read_without_asking() {
    let setup = Setup::new().await;
    let unknown = ProjectId::parse("0199e0f2-0000-7000-8000-0000000000ff").unwrap();
    let reading = |project: &ProjectId, folders: Vec<PathBuf>| {
        setup.git(&["--version"]).checked_as(ReadPlace::Project {
            project: project.clone(),
            folders,
        })
    };

    let inside = setup
        .processes
        .run(reading(&setup.project, vec![setup.root.clone()]))
        .await;
    let outside = setup
        .processes
        .run(reading(&setup.project, vec![setup.outside.clone()]))
        .await;
    let of_unknown = setup
        .processes
        .run(reading(&unknown, vec![setup.root.clone()]))
        .await;
    let folder = setup
        .processes
        .project_folder(setup.project.clone())
        .await
        .unwrap();
    let unknown_folder = setup.processes.project_folder(unknown.clone()).await;

    assert!(inside.unwrap().succeeded());
    assert_eq!(
        outside,
        Err(ProcessError::ReadNotAllowed {
            path: setup.outside.display().to_string()
        })
    );
    assert_eq!(of_unknown, Err(ProcessError::ProjectNotFound));
    assert_eq!(folder, setup.root);
    assert_eq!(unknown_folder, Err(ProcessError::ProjectNotFound));
}
