use std::path::PathBuf;

use mannie_types::ceiling::CeilingMode;
use mannie_types::clock::UtcMillis;
use mannie_types::ids::{ProjectId, RuleId};
use mannie_types::tools::{
    Access, AccessMode, ApprovalReason, ApprovalRequest, CommandRule, Decision, EntryCount,
    GrantScope, Refusal, RemoteCommandRule, RemoteTarget, RuleProposal, ToolsError,
};

use super::{Grants, PolicyInputs, SavedRule, Verdict, decide, saved_rule};
use crate::ceiling::Ceiling;
use crate::digest::digest;

/// An absolute path on this platform: `C:\work` on Windows, `/work` elsewhere.
fn at(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

fn path_access(path: &str, mode: AccessMode, folder: bool) -> Access {
    Access::Path {
        path: at(path),
        mode,
        folder,
        exists: mode != AccessMode::Create,
        identity: "1:1".to_owned(),
        entries: None,
    }
}

fn read(path: &str) -> Access {
    path_access(path, AccessMode::Read, false)
}

fn write(path: &str) -> Access {
    path_access(path, AccessMode::Write, false)
}

fn command(text: &str, directory: &str) -> Access {
    Access::Command {
        command: text.to_owned(),
        shell: at("/bin/sh"),
        working_directory: at(directory),
    }
}

fn path_grant(mode: AccessMode, folder: &str) -> GrantScope {
    GrantScope::Path {
        mode,
        folder: at(folder),
    }
}

fn command_grant(text: &str, directory: &str) -> GrantScope {
    GrantScope::Command {
        command: text.to_owned(),
        working_directory: at(directory),
    }
}

fn rule(words: &[&str], folder: Option<&str>) -> CommandRule {
    CommandRule {
        id: RuleId::parse("0199e0f2-0000-7000-9000-000000000001").unwrap(),
        words: words.iter().map(|word| (*word).to_owned()).collect(),
        folder: folder.map(at),
        created_at: UtcMillis::EPOCH,
    }
}

/// The inputs of the policy, built up by each test.
struct Inputs {
    workspace: Option<PathBuf>,
    allow_changes_in_workspace: bool,
    protected: Vec<PathBuf>,
    grants: Vec<GrantScope>,
    project_grants: Vec<GrantScope>,
    always_grants: Vec<GrantScope>,
    rules: Vec<CommandRule>,
    remote_rules: Vec<RemoteCommandRule>,
    project: Option<ProjectId>,
    owner_request: Option<String>,
    ceiling: Option<Ceiling>,
    host: Option<String>,
    full_access: bool,
}

impl Inputs {
    /// A workspace at `/work`, changes asking, Mannie's data in `/data`.
    fn new() -> Self {
        Self {
            workspace: Some(PathBuf::from(at("/work"))),
            allow_changes_in_workspace: false,
            protected: vec![PathBuf::from(at("/data/mannie"))],
            grants: Vec::new(),
            project_grants: Vec::new(),
            always_grants: Vec::new(),
            rules: Vec::new(),
            remote_rules: Vec::new(),
            project: None,
            owner_request: None,
            ceiling: None,
            host: None,
            full_access: false,
        }
    }

    /// A worker named `build-01` whose ceiling allows `/srv/app` and the
    /// command rule `cargo test`, with its data in `/var/lib/mannie-worker`.
    fn worker(mode: CeilingMode) -> Self {
        Self {
            workspace: Some(PathBuf::from(at("/srv/app"))),
            protected: vec![PathBuf::from(at("/var/lib/mannie-worker"))],
            ceiling: Some(Ceiling {
                mode,
                allowed_folders: vec![PathBuf::from(at("/srv/app"))],
                rules: vec![rule(&["cargo", "test"], None)],
            }),
            host: Some("build-01".to_owned()),
            ..Self::new()
        }
    }

    fn decide(&self, accesses: &[Access]) -> Verdict {
        decide(
            accesses,
            &PolicyInputs {
                workspace: self.workspace.as_deref(),
                allow_changes_in_workspace: self.allow_changes_in_workspace,
                protected: &self.protected,
                grants: Grants {
                    conversation: &self.grants,
                    project: &self.project_grants,
                    always: &self.always_grants,
                },
                rules: &self.rules,
                remote_rules: &self.remote_rules,
                project: self.project.as_ref(),
                owner_request: self.owner_request.as_deref(),
                ceiling: self.ceiling.as_ref(),
                host: self.host.as_deref(),
                full_access: self.full_access,
            },
        )
    }
}

fn ask(
    accesses: &[Access],
    reasons: &[ApprovalReason],
    grants: Vec<GrantScope>,
    rule: Option<RuleProposal>,
) -> Verdict {
    Verdict::Ask(Box::new(ApprovalRequest {
        digest: digest(accesses),
        reasons: reasons.to_vec(),
        grants,
        rule,
        project: None,
        host: None,
    }))
}

/// A worker's request: it names the host and offers nothing for longer than
/// once.
fn worker_ask(accesses: &[Access], reasons: &[ApprovalReason]) -> Verdict {
    Verdict::Ask(Box::new(ApprovalRequest {
        digest: digest(accesses),
        reasons: reasons.to_vec(),
        grants: Vec::new(),
        rule: None,
        project: None,
        host: Some("build-01".to_owned()),
    }))
}

const MODES: [CeilingMode; 3] = [
    CeilingMode::ReadOnly,
    CeilingMode::Ask,
    CeilingMode::Autonomous,
];

fn project_id() -> ProjectId {
    ProjectId::parse("0199e0f2-0000-7000-a000-000000000001").unwrap()
}

fn move_into(name: &str, folder: &str) -> Access {
    Access::Project {
        id: project_id(),
        name: name.to_owned(),
        folder: at(folder),
    }
}

#[test]
fn a_call_without_accesses_needs_no_authorization() {
    assert_eq!(Inputs::new().decide(&[]), Verdict::Unrestricted);
}

#[test]
fn reads_inside_the_workspace_are_allowed() {
    let inputs = Inputs::new();

    assert_eq!(
        inputs.decide(&[read("/work/src/main.rs")]),
        Verdict::Allowed(Decision::Workspace)
    );
    assert_eq!(
        inputs.decide(&[path_access("/work", AccessMode::Read, true)]),
        Verdict::Allowed(Decision::Workspace)
    );
}

#[test]
fn changes_inside_the_workspace_ask_unless_the_settings_allow_them() {
    let mut inputs = Inputs::new();
    let accesses = [write("/work/src/main.rs")];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::ChangeInWorkspace],
            vec![path_grant(AccessMode::Write, "/work/src")],
            None
        )
    );

    inputs.allow_changes_in_workspace = true;
    assert_eq!(
        inputs.decide(&accesses),
        Verdict::Allowed(Decision::Workspace)
    );
}

#[test]
fn paths_outside_the_workspace_ask_and_grants_allow_them() {
    let mut inputs = Inputs::new();
    let accesses = [read("/other/notes.txt")];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::OutsideWorkspace],
            vec![path_grant(AccessMode::Read, "/other")],
            None
        )
    );

    inputs.grants = vec![path_grant(AccessMode::Write, "/other")];
    assert!(matches!(inputs.decide(&accesses), Verdict::Ask(_)));

    inputs.grants = vec![path_grant(AccessMode::Read, "/other")];
    assert_eq!(inputs.decide(&accesses), Verdict::Allowed(Decision::Grant));
    assert_eq!(
        inputs.decide(&[read("/other/deeper/notes.txt")]),
        Verdict::Allowed(Decision::Grant)
    );
    assert!(matches!(
        inputs.decide(&[read("/otherwise/notes.txt")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn without_a_workspace_every_path_asks() {
    let mut inputs = Inputs::new();
    inputs.workspace = None;
    inputs.allow_changes_in_workspace = true;
    let accesses = [read("/work/notes.txt")];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::NoWorkspace],
            vec![path_grant(AccessMode::Read, "/work")],
            None
        )
    );
}

#[test]
fn mannies_folders_always_ask_and_offer_no_grant() {
    let mut inputs = Inputs::new();
    // A workspace that contains Mannie's data, with every source allowing.
    inputs.workspace = Some(PathBuf::from(at("/")));
    inputs.allow_changes_in_workspace = true;
    inputs.grants = vec![
        path_grant(AccessMode::Write, "/"),
        command_grant("sqlite3 mannie.db", "/data/mannie"),
    ];
    inputs.rules = vec![rule(&["sqlite3"], None)];
    let database = [write("/data/mannie/mannie.db")];
    let shell = [command("sqlite3 mannie.db", "/data/mannie")];

    assert_eq!(
        inputs.decide(&database),
        ask(&database, &[ApprovalReason::MannieFolder], Vec::new(), None)
    );
    assert_eq!(
        inputs.decide(&shell),
        ask(&shell, &[ApprovalReason::MannieFolder], Vec::new(), None)
    );
    assert!(matches!(
        inputs.decide(&[read("/data/mannie/logs/today.log")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn commands_ask_and_offer_a_grant_and_a_rule() {
    let inputs = Inputs::new();
    let accesses = [command("git status --short", "/work")];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::Command],
            vec![command_grant("git status --short", "/work")],
            Some(RuleProposal {
                words: vec!["git".to_owned(), "status".to_owned(), "--short".to_owned()],
                folder: at("/work"),
                remote: None,
            })
        )
    );
}

#[test]
fn the_workspace_never_allows_a_command() {
    let mut inputs = Inputs::new();
    inputs.allow_changes_in_workspace = true;

    assert!(matches!(
        inputs.decide(&[command("cargo build", "/work")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn rules_allow_commands_that_start_with_their_words() {
    let mut inputs = Inputs::new();
    inputs.rules = vec![rule(&["git", "status"], None)];

    assert_eq!(
        inputs.decide(&[command("git status --short", "/elsewhere")]),
        Verdict::Allowed(Decision::Rule)
    );
    assert!(matches!(
        inputs.decide(&[command("git stash", "/work")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn a_rule_limited_to_a_folder_needs_the_working_directory_inside_it() {
    let mut inputs = Inputs::new();
    inputs.rules = vec![rule(&["cargo", "test"], Some("/work"))];

    assert_eq!(
        inputs.decide(&[command("cargo test", "/work/crates/tools")]),
        Verdict::Allowed(Decision::Rule)
    );
    assert!(matches!(
        inputs.decide(&[command("cargo test", "/other")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn grants_of_commands_need_the_same_text_and_directory() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![command_grant("npm test", "/work")];

    assert_eq!(
        inputs.decide(&[command("npm test", "/work")]),
        Verdict::Allowed(Decision::Grant)
    );
    assert!(matches!(
        inputs.decide(&[command("npm test", "/work/app")]),
        Verdict::Ask(_)
    ));
    assert!(matches!(
        inputs.decide(&[command("npm test -- --watch", "/work")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn commands_with_metacharacters_are_never_allowed_by_grants_or_rules() {
    let mut inputs = Inputs::new();
    let text = "git status; rm -rf work";
    inputs.rules = vec![rule(&["git", "status"], None), rule(&["git"], None)];
    inputs.grants = vec![command_grant(text, "/work")];
    let accesses = [command(text, "/work")];

    assert_eq!(
        inputs.decide(&accesses),
        ask(&accesses, &[ApprovalReason::Command], Vec::new(), None)
    );
}

#[test]
fn an_operation_is_allowed_only_when_every_access_is() {
    let mut inputs = Inputs::new();
    let copy = [
        path_access("/work/src", AccessMode::Read, true),
        path_access("/other/src", AccessMode::Create, true),
    ];

    // Only the access that asked is granted.
    assert_eq!(
        inputs.decide(&copy),
        ask(
            &copy,
            &[ApprovalReason::OutsideWorkspace],
            vec![path_grant(AccessMode::Create, "/other/src")],
            None
        )
    );

    inputs.grants = vec![path_grant(AccessMode::Create, "/other")];
    assert_eq!(inputs.decide(&copy), Verdict::Allowed(Decision::Grant));
}

#[test]
fn grants_cover_a_folder_itself_and_the_folder_of_a_file() {
    let inputs = Inputs::new();
    let delete = [Access::Path {
        path: at("/other/build"),
        mode: AccessMode::Delete,
        folder: true,
        exists: true,
        identity: "1:2".to_owned(),
        entries: Some(EntryCount {
            files: 10,
            folders: 2,
        }),
    }];
    let create_folders = [
        path_access("/other/new", AccessMode::Create, true),
        path_access("/other/new/deeper", AccessMode::Create, true),
    ];

    let Verdict::Ask(request) = inputs.decide(&delete) else {
        panic!("a delete outside the workspace asks");
    };
    assert_eq!(
        request.grants,
        vec![path_grant(AccessMode::Delete, "/other/build")]
    );
    // The folder below is covered by the grant of the folder above it.
    let Verdict::Ask(request) = inputs.decide(&create_folders) else {
        panic!("creating folders outside the workspace asks");
    };
    assert_eq!(
        request.grants,
        vec![path_grant(AccessMode::Create, "/other/new")]
    );
}

#[test]
fn every_reason_is_named_once() {
    let mut inputs = Inputs::new();
    inputs.workspace = Some(PathBuf::from(at("/work")));
    let accesses = [
        write("/work/a.txt"),
        write("/work/b.txt"),
        read("/other/c.txt"),
    ];

    let Verdict::Ask(request) = inputs.decide(&accesses) else {
        panic!("changes and reads outside the workspace ask");
    };
    assert_eq!(
        request.reasons,
        vec![
            ApprovalReason::ChangeInWorkspace,
            ApprovalReason::OutsideWorkspace
        ]
    );
}

#[test]
fn a_decision_names_its_most_specific_source() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![path_grant(AccessMode::Read, "/other")];

    assert_eq!(
        inputs.decide(&[read("/work/a.txt"), read("/other/b.txt")]),
        Verdict::Allowed(Decision::Grant)
    );
}

#[test]
fn grants_of_a_project_and_for_everyone_allow_what_they_cover() {
    let mut inputs = Inputs::new();
    let write = write("/other/a.txt");
    inputs.project_grants = vec![path_grant(AccessMode::Write, "/other")];
    assert_eq!(
        inputs.decide(std::slice::from_ref(&write)),
        Verdict::Allowed(Decision::ProjectGrant)
    );

    inputs.project_grants.clear();
    inputs.always_grants = vec![command_grant("npm test", "/work")];
    assert_eq!(
        inputs.decide(&[command("npm test", "/work")]),
        Verdict::Allowed(Decision::AlwaysGrant)
    );
    assert!(matches!(inputs.decide(&[write]), Verdict::Ask(_)));
}

#[test]
fn a_decision_names_the_grant_that_reaches_furthest() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![path_grant(AccessMode::Read, "/other")];
    inputs.always_grants = vec![path_grant(AccessMode::Read, "/elsewhere")];

    assert_eq!(
        inputs.decide(&[read("/other/a.txt"), read("/elsewhere/b.txt")]),
        Verdict::Allowed(Decision::AlwaysGrant)
    );
}

#[test]
fn a_request_names_the_project_that_its_grants_would_cover() {
    let mut inputs = Inputs::new();
    inputs.project = Some(project_id());
    let accesses = [write("/other/a.txt")];

    let Verdict::Ask(request) = inputs.decide(&accesses) else {
        panic!("the write does not ask");
    };

    assert_eq!(request.project, Some(project_id()));
    assert_eq!(
        request.grants,
        vec![path_grant(AccessMode::Write, "/other")]
    );
}

#[test]
fn a_move_into_a_project_asks_unless_the_owners_request_names_it() {
    let mut inputs = Inputs::new();
    inputs.workspace = None;
    let accesses = [move_into("mannie", "/projects/mannie")];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::MoveToProject],
            Vec::new(),
            None
        )
    );

    inputs.owner_request = Some("Fix the tests in Mannie, please.".to_owned());
    assert_eq!(
        inputs.decide(&accesses),
        Verdict::Allowed(Decision::OwnerNamedProject)
    );

    inputs.owner_request = Some("Fix the tests in the other app.".to_owned());
    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::MoveToProject],
            Vec::new(),
            None
        )
    );
}

#[test]
fn no_grant_or_rule_moves_a_conversation() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![path_grant(AccessMode::Read, "/")];
    inputs.always_grants = vec![path_grant(AccessMode::Write, "/")];
    let accesses = [move_into("mannie", "/projects/mannie")];

    let Verdict::Ask(request) = inputs.decide(&accesses) else {
        panic!("the move does not ask");
    };

    assert_eq!(request.grants, Vec::new());
    assert_eq!(request.rule, None);
}

#[cfg(windows)]
#[test]
fn windows_paths_compare_without_regard_to_case() {
    let mut inputs = Inputs::new();
    inputs.protected = vec![PathBuf::from(r"C:\Users\Owner\AppData\Local\Mannie")];

    assert_eq!(
        inputs.decide(&[read("/WORK/Notes.txt")]),
        Verdict::Allowed(Decision::Workspace)
    );
    assert!(matches!(
        inputs.decide(&[Access::Path {
            path: r"c:\users\owner\appdata\local\mannie\mannie.db".to_owned(),
            mode: AccessMode::Read,
            folder: false,
            exists: true,
            identity: "1:3".to_owned(),
            entries: None,
        }]),
        Verdict::Ask(_)
    ));
}

#[test]
fn saved_rules_keep_the_chosen_leading_words() {
    let proposal = RuleProposal {
        words: vec!["git".to_owned(), "status".to_owned(), "--short".to_owned()],
        folder: at("/work"),
        remote: None,
    };

    assert_eq!(
        saved_rule(&proposal, 2, false),
        Ok(SavedRule::Local {
            words: vec!["git".to_owned(), "status".to_owned()],
            folder: None
        })
    );
    assert_eq!(
        saved_rule(&proposal, 3, true),
        Ok(SavedRule::Local {
            words: proposal.words.clone(),
            folder: Some(at("/work"))
        })
    );
    assert_eq!(
        saved_rule(&proposal, 0, false),
        Err(ToolsError::RulePrefixInvalid)
    );
    assert_eq!(
        saved_rule(&proposal, 4, false),
        Err(ToolsError::RulePrefixInvalid)
    );
}

// The ceiling of a worker (plugins/workers/docs/host-and-ceiling.md, the
// ceiling): one test for each row of its table.

#[test]
fn a_read_inside_the_allowed_folders_runs_in_every_mode() {
    for mode in MODES {
        assert_eq!(
            Inputs::worker(mode).decide(&[read("/srv/app/src/main.rs")]),
            Verdict::Allowed(Decision::AllowedFolder),
            "{mode:?}"
        );
    }
}

#[test]
fn a_read_outside_the_allowed_folders_asks_in_every_mode() {
    for mode in MODES {
        let accesses = [read("/etc/hosts")];
        assert_eq!(
            Inputs::worker(mode).decide(&accesses),
            worker_ask(&accesses, &[ApprovalReason::OutsideAllowedFolders]),
            "{mode:?}"
        );
    }
}

#[test]
fn read_only_refuses_every_change_and_every_command() {
    let inputs = Inputs::worker(CeilingMode::ReadOnly);

    for accesses in [
        vec![write("/srv/app/a.txt")],
        vec![write("/tmp/a.txt")],
        vec![command("cargo test", "/srv/app")],
        vec![command("ls", "/srv/app")],
    ] {
        assert_eq!(
            inputs.decide(&accesses),
            Verdict::Refused(Refusal::ReadOnlyHost),
            "{accesses:?}"
        );
    }
}

#[test]
fn ask_asks_for_every_change_and_every_command() {
    let inputs = Inputs::worker(CeilingMode::Ask);
    let inside = [write("/srv/app/a.txt")];
    let outside = [write("/tmp/a.txt")];
    let ruled = [command("cargo test", "/srv/app")];

    assert_eq!(
        inputs.decide(&inside),
        worker_ask(&inside, &[ApprovalReason::HostAsks])
    );
    assert_eq!(
        inputs.decide(&outside),
        worker_ask(&outside, &[ApprovalReason::OutsideAllowedFolders])
    );
    assert_eq!(
        inputs.decide(&ruled),
        worker_ask(&ruled, &[ApprovalReason::Command])
    );
}

#[test]
fn autonomous_runs_changes_inside_the_allowed_folders_and_commands_that_a_host_rule_allows() {
    let inputs = Inputs::worker(CeilingMode::Autonomous);
    let outside = [write("/tmp/a.txt")];
    let unruled = [command("cargo build", "/srv/app")];
    let chained = [command("cargo test; rm -rf /srv/app", "/srv/app")];

    assert_eq!(
        inputs.decide(&[write("/srv/app/a.txt")]),
        Verdict::Allowed(Decision::AllowedFolder)
    );
    assert_eq!(
        inputs.decide(&[command("cargo test --workspace", "/srv/app")]),
        Verdict::Allowed(Decision::HostRule)
    );
    assert_eq!(
        inputs.decide(&outside),
        worker_ask(&outside, &[ApprovalReason::OutsideAllowedFolders])
    );
    assert_eq!(
        inputs.decide(&unruled),
        worker_ask(&unruled, &[ApprovalReason::Command])
    );
    // A rule never covers shell metacharacters.
    assert_eq!(
        inputs.decide(&chained),
        worker_ask(&chained, &[ApprovalReason::Command])
    );
}

#[test]
fn the_workers_own_folders_always_ask_and_read_only_refuses_changes_there() {
    let read_data = [read("/var/lib/mannie-worker/link-key")];
    let write_data = [write("/var/lib/mannie-worker/master-pin")];
    for mode in MODES {
        assert_eq!(
            Inputs::worker(mode).decide(&read_data),
            worker_ask(&read_data, &[ApprovalReason::MannieFolder]),
            "{mode:?}"
        );
    }
    assert_eq!(
        Inputs::worker(CeilingMode::Autonomous).decide(&write_data),
        worker_ask(&write_data, &[ApprovalReason::MannieFolder])
    );
    assert_eq!(
        Inputs::worker(CeilingMode::ReadOnly).decide(&write_data),
        Verdict::Refused(Refusal::ReadOnlyHost)
    );
}

#[test]
fn one_refused_access_refuses_the_whole_operation() {
    let accesses = [read("/srv/app/a.txt"), write("/srv/app/b.txt")];

    assert_eq!(
        Inputs::worker(CeilingMode::ReadOnly).decide(&accesses),
        Verdict::Refused(Refusal::ReadOnlyHost)
    );
}

#[test]
fn a_move_into_a_project_is_refused_on_a_worker() {
    for mode in MODES {
        assert_eq!(
            Inputs::worker(mode).decide(&[move_into("Mannie", "/srv/app")]),
            Verdict::Refused(Refusal::NoProjectsOnHost),
            "{mode:?}"
        );
    }
}

#[test]
fn a_task_narrows_the_ceiling_and_never_widens_it() {
    let mut inputs = Inputs::worker(CeilingMode::Autonomous);
    let ceiling = inputs.ceiling.clone().unwrap();
    inputs.ceiling = Some(ceiling.narrowed(Some(CeilingMode::ReadOnly)));

    assert_eq!(
        inputs.decide(&[write("/srv/app/a.txt")]),
        Verdict::Refused(Refusal::ReadOnlyHost)
    );

    let mut asking = Inputs::worker(CeilingMode::Ask);
    let ceiling = asking.ceiling.clone().unwrap();
    asking.ceiling = Some(ceiling.narrowed(Some(CeilingMode::Autonomous)));
    let accesses = [write("/srv/app/a.txt")];

    assert_eq!(
        asking.decide(&accesses),
        worker_ask(&accesses, &[ApprovalReason::HostAsks])
    );
}

#[test]
fn a_host_rule_limited_to_a_folder_runs_only_there() {
    let mut inputs = Inputs::worker(CeilingMode::Autonomous);
    if let Some(ceiling) = &mut inputs.ceiling {
        ceiling.rules = vec![rule(&["make"], Some("/srv/app"))];
    }
    let elsewhere = [command("make", "/tmp")];

    assert_eq!(
        inputs.decide(&[command("make", "/srv/app/sub")]),
        Verdict::Allowed(Decision::HostRule)
    );
    assert_eq!(
        inputs.decide(&elsewhere),
        worker_ask(&elsewhere, &[ApprovalReason::Command])
    );
}

#[test]
fn the_masters_rules_and_workspace_setting_do_not_apply_on_a_worker() {
    let mut inputs = Inputs::worker(CeilingMode::Ask);
    inputs.rules = vec![rule(&["ls"], None)];
    inputs.allow_changes_in_workspace = true;
    let listing = [command("ls", "/srv/app")];
    let change = [write("/srv/app/a.txt")];

    assert_eq!(
        inputs.decide(&listing),
        worker_ask(&listing, &[ApprovalReason::Command])
    );
    assert_eq!(
        inputs.decide(&change),
        worker_ask(&change, &[ApprovalReason::HostAsks])
    );
}

#[test]
fn full_access_allows_every_operation_on_the_master() {
    let mut inputs = Inputs::new();
    inputs.full_access = true;

    for accesses in [
        vec![read("/elsewhere/notes.txt")],
        vec![write("/work/src/main.rs")],
        vec![write("/data/mannie/mannie.sqlite")],
        vec![command("rm -rf build && cargo build", "/work")],
        vec![move_into("app", "/projects/app")],
        vec![read("/work/a.txt"), command("make", "/data/mannie")],
    ] {
        assert_eq!(
            inputs.decide(&accesses),
            Verdict::Allowed(Decision::FullAccess),
            "{accesses:?}"
        );
    }
}

#[test]
fn a_conversations_full_access_is_no_input_of_a_workers_ceiling() {
    let mut inputs = Inputs::worker(CeilingMode::Ask);
    inputs.full_access = true;
    let listing = [command("ls", "/srv/app")];

    assert_eq!(
        inputs.decide(&listing),
        worker_ask(&listing, &[ApprovalReason::Command])
    );
}

#[test]
fn a_host_in_full_access_runs_every_path_and_command() {
    let inputs = Inputs::worker(CeilingMode::FullAccess);

    for accesses in [
        vec![read("/etc/hosts")],
        vec![write("/srv/app/a.txt")],
        vec![write("/home/someone/b.txt")],
        vec![write("/var/lib/mannie-worker/mannie-worker.sqlite")],
        vec![command("ls | wc -l", "/tmp")],
        vec![command("cargo build", "/var/lib/mannie-worker")],
    ] {
        assert_eq!(
            inputs.decide(&accesses),
            Verdict::Allowed(Decision::HostFullAccess),
            "{accesses:?}"
        );
    }
    assert_eq!(
        inputs.decide(&[move_into("app", "/projects/app")]),
        Verdict::Refused(Refusal::NoProjectsOnHost)
    );
}

#[test]
fn a_read_only_task_narrows_a_host_in_full_access() {
    let inputs = Inputs::worker(CeilingMode::FullAccess.narrowed_by(Some(CeilingMode::ReadOnly)));

    assert_eq!(
        inputs.decide(&[read("/srv/app/a.txt")]),
        Verdict::Allowed(Decision::AllowedFolder)
    );
    assert_eq!(
        inputs.decide(&[write("/srv/app/a.txt")]),
        Verdict::Refused(Refusal::ReadOnlyHost)
    );
    assert_eq!(
        inputs.decide(&[command("ls", "/srv/app")]),
        Verdict::Refused(Refusal::ReadOnlyHost)
    );
}

// Commands on remote hosts (ADR-0036): the same choices as commands here,
// with grants and rules that cover one host at one endpoint.

const SSH_HOST: &str = "0199e0f2-0000-7000-8000-0000000000aa";
const OTHER_HOST: &str = "0199e0f2-0000-7000-8000-0000000000bb";
const ENDPOINT: &str = "deploy@build-box.lan:22";

fn remote_on(target: &str, endpoint: &str, text: &str) -> Access {
    Access::RemoteCommand {
        plugin: "ssh".to_owned(),
        target: target.to_owned(),
        label: "build-box".to_owned(),
        endpoint: endpoint.to_owned(),
        identity: "SHA256:vh3fjoyg7pBnxLhORIqZjyJQqYYN/gzBTuj+mPdeJlo".to_owned(),
        command: text.to_owned(),
    }
}

fn remote(text: &str) -> Access {
    remote_on(SSH_HOST, ENDPOINT, text)
}

fn host(target: &str, endpoint: &str) -> RemoteTarget {
    RemoteTarget {
        plugin: "ssh".to_owned(),
        target: target.to_owned(),
        label: "build-box".to_owned(),
        endpoint: endpoint.to_owned(),
    }
}

fn remote_grant(text: &str) -> GrantScope {
    GrantScope::RemoteCommand {
        plugin: "ssh".to_owned(),
        target: SSH_HOST.to_owned(),
        label: "build-box".to_owned(),
        endpoint: ENDPOINT.to_owned(),
        command: text.to_owned(),
    }
}

fn remote_rule(target: &str, words: &[&str]) -> RemoteCommandRule {
    RemoteCommandRule {
        id: RuleId::parse("0199e0f2-0000-7000-9000-000000000002").unwrap(),
        target: host(target, ENDPOINT),
        words: words.iter().map(|word| (*word).to_owned()).collect(),
        created_at: UtcMillis::EPOCH,
    }
}

#[test]
fn a_remote_command_asks_and_offers_a_grant_and_a_rule_for_its_host() {
    let inputs = Inputs::new();
    let accesses = [remote("systemctl status nginx")];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::RemoteCommand],
            vec![remote_grant("systemctl status nginx")],
            Some(RuleProposal {
                words: vec![
                    "systemctl".to_owned(),
                    "status".to_owned(),
                    "nginx".to_owned()
                ],
                folder: String::new(),
                remote: Some(host(SSH_HOST, ENDPOINT)),
            })
        )
    );
}

#[test]
fn neither_the_workspace_nor_a_local_rule_or_grant_allows_a_remote_command() {
    let mut inputs = Inputs::new();
    inputs.allow_changes_in_workspace = true;
    inputs.rules = vec![rule(&["uptime"], None)];
    inputs.grants = vec![command_grant("uptime", "/work")];

    assert!(matches!(
        inputs.decide(&[remote("uptime")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn a_grant_covers_the_same_command_on_the_same_host_and_endpoint_only() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![remote_grant("uptime")];

    assert_eq!(
        inputs.decide(&[remote("uptime")]),
        Verdict::Allowed(Decision::Grant)
    );
    for accesses in [
        [remote("uptime -p")],
        [remote_on(OTHER_HOST, ENDPOINT, "uptime")],
        [remote_on(SSH_HOST, "root@build-box.lan:22", "uptime")],
        [remote_on(SSH_HOST, "deploy@build-box.lan:2222", "uptime")],
    ] {
        assert!(
            matches!(inputs.decide(&accesses), Verdict::Ask(_)),
            "{accesses:?}"
        );
    }

    // The host's name is for display: a renamed host keeps its grants.
    let mut renamed = remote("uptime");
    if let Access::RemoteCommand { label, .. } = &mut renamed {
        "build-box-2".clone_into(label);
    }
    assert_eq!(inputs.decide(&[renamed]), Verdict::Allowed(Decision::Grant));

    let mut shared = Inputs::new();
    shared.project_grants = vec![remote_grant("uptime")];
    assert_eq!(
        shared.decide(&[remote("uptime")]),
        Verdict::Allowed(Decision::ProjectGrant)
    );
    shared.project_grants = Vec::new();
    shared.always_grants = vec![remote_grant("uptime")];
    assert_eq!(
        shared.decide(&[remote("uptime")]),
        Verdict::Allowed(Decision::AlwaysGrant)
    );
}

#[test]
fn a_remote_rule_allows_leading_words_on_its_host_only() {
    let mut inputs = Inputs::new();
    inputs.remote_rules = vec![remote_rule(SSH_HOST, &["systemctl", "status"])];
    let elsewhere = [remote_on(OTHER_HOST, ENDPOINT, "systemctl status nginx")];

    assert_eq!(
        inputs.decide(&[remote("systemctl status nginx")]),
        Verdict::Allowed(Decision::Rule)
    );
    assert!(matches!(inputs.decide(&elsewhere), Verdict::Ask(_)));
    assert!(matches!(
        inputs.decide(&[remote("systemctl restart nginx")]),
        Verdict::Ask(_)
    ));
    // A rule of a remote host never matches a command here.
    assert!(matches!(
        inputs.decide(&[command("systemctl status nginx", "/work")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn a_remote_command_with_metacharacters_or_a_backslash_can_only_be_allowed_once() {
    let mut inputs = Inputs::new();
    inputs.remote_rules = vec![remote_rule(SSH_HOST, &["uptime"])];
    for text in [
        "uptime; reboot",
        "uptime && reboot",
        r"uptime\ -p",
        "uptime $(reboot)",
    ] {
        let accesses = [remote(text)];
        assert_eq!(
            inputs.decide(&accesses),
            ask(
                &accesses,
                &[ApprovalReason::RemoteCommand],
                Vec::new(),
                None
            ),
            "{text:?}"
        );
    }
}

#[test]
fn full_access_allows_a_remote_command() {
    let mut inputs = Inputs::new();
    inputs.full_access = true;

    assert_eq!(
        inputs.decide(&[remote("reboot; rm -rf /")]),
        Verdict::Allowed(Decision::FullAccess)
    );
}

#[test]
fn a_worker_refuses_a_remote_command_in_every_mode() {
    for mode in MODES.into_iter().chain([CeilingMode::FullAccess]) {
        assert_eq!(
            Inputs::worker(mode).decide(&[remote("uptime")]),
            Verdict::Refused(Refusal::NoRemoteCommands),
            "{mode:?}"
        );
    }
}

#[test]
fn a_saved_remote_rule_is_always_limited_to_its_host() {
    let proposal = RuleProposal {
        words: vec![
            "systemctl".to_owned(),
            "status".to_owned(),
            "nginx".to_owned(),
        ],
        folder: String::new(),
        remote: Some(host(SSH_HOST, ENDPOINT)),
    };

    for limit_to_folder in [false, true] {
        assert_eq!(
            saved_rule(&proposal, 2, limit_to_folder),
            Ok(SavedRule::Remote {
                target: host(SSH_HOST, ENDPOINT),
                words: vec!["systemctl".to_owned(), "status".to_owned()],
            })
        );
    }
    assert_eq!(
        saved_rule(&proposal, 0, false),
        Err(ToolsError::RulePrefixInvalid)
    );
}
