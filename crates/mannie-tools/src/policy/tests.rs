use std::path::PathBuf;

use mannie_types::ceiling::CeilingMode;
use mannie_types::clock::UtcMillis;
use mannie_types::ids::{DatabaseKey, ProjectId, RuleId, WorkerId};
use mannie_types::tools::{
    Access, AccessMode, ApprovalReason, ApprovalRequest, CommandRule, DatabaseHome, Decision,
    DiscardedWork, EntryCount, GrantScope, ProgramEffect, ProgramRule, ProgramTarget, Refusal,
    RemoteCommandRule, RemoteTarget, RuleProposal, StatementClass, TerminalCommandRule,
    TerminalHost, TerminalTarget, ToolHint, ToolServerEndpoint, ToolsError, effect_keys,
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
        state: None,
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
        background: false,
    }
}

/// The same access, for a command that runs in the background (ADR-0041).
fn in_background(mut access: Access) -> Access {
    match &mut access {
        Access::Command { background, .. } | Access::RemoteCommand { background, .. } => {
            *background = true;
        }
        other => panic!("only commands run in the background: {other:?}"),
    }
    access
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
        background: false,
    }
}

/// The same grant, given from a command in the background, so it covers both
/// modes (D40).
fn background_grant(mut grant: GrantScope) -> GrantScope {
    match &mut grant {
        GrantScope::Command { background, .. } | GrantScope::RemoteCommand { background, .. } => {
            *background = true;
        }
        other => panic!("only command grants have a mode: {other:?}"),
    }
    grant
}

fn rule(words: &[&str], folder: Option<&str>) -> CommandRule {
    CommandRule {
        id: RuleId::parse("0199e0f2-0000-7000-9000-000000000001").unwrap(),
        words: words.iter().map(|word| (*word).to_owned()).collect(),
        folder: folder.map(at),
        created_at: UtcMillis::EPOCH,
        background: false,
    }
}

/// The inputs of the policy, built up by each test.
struct Inputs {
    workspace: Option<PathBuf>,
    allow_changes_in_workspace: bool,
    protected: Vec<PathBuf>,
    skill_folders: Vec<PathBuf>,
    grants: Vec<GrantScope>,
    project_grants: Vec<GrantScope>,
    always_grants: Vec<GrantScope>,
    rules: Vec<CommandRule>,
    remote_rules: Vec<RemoteCommandRule>,
    program_rules: Vec<ProgramRule>,
    terminal_rules: Vec<TerminalCommandRule>,
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
            skill_folders: Vec::new(),
            grants: Vec::new(),
            project_grants: Vec::new(),
            always_grants: Vec::new(),
            rules: Vec::new(),
            remote_rules: Vec::new(),
            program_rules: Vec::new(),
            terminal_rules: Vec::new(),
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
                skill_folders: &self.skill_folders,
                grants: Grants {
                    conversation: &self.grants,
                    project: &self.project_grants,
                    always: &self.always_grants,
                },
                rules: &self.rules,
                remote_rules: &self.remote_rules,
                program_rules: &self.program_rules,
                terminal_rules: &self.terminal_rules,
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
                program: None,
                background: false,
                terminal: None,
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
        state: None,
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
            state: None,
        }]),
        Verdict::Ask(_)
    ));
}

// Commands in the background (ADR-0041, D40): a grant or a rule given from a
// command in the background covers both modes; one from a command in the
// foreground still asks before a command in the background.

#[test]
fn a_command_in_the_background_asks_and_offers_a_grant_and_a_rule_for_both_modes() {
    let inputs = Inputs::new();
    let accesses = [in_background(command("npm run dev", "/work"))];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::Command],
            vec![background_grant(command_grant("npm run dev", "/work"))],
            Some(RuleProposal {
                words: vec!["npm".to_owned(), "run".to_owned(), "dev".to_owned()],
                folder: at("/work"),
                remote: None,
                program: None,
                background: true,
                terminal: None,
            })
        )
    );
}

#[test]
fn a_grant_or_a_rule_from_the_foreground_still_asks_before_the_background() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![command_grant("npm run dev", "/work")];
    inputs.rules = vec![rule(&["cargo", "watch"], None)];
    let granted = [in_background(command("npm run dev", "/work"))];
    let ruled = [in_background(command("cargo watch -x test", "/work"))];

    assert!(matches!(inputs.decide(&granted), Verdict::Ask(_)));
    assert!(matches!(inputs.decide(&ruled), Verdict::Ask(_)));
    assert_eq!(
        inputs.decide(&[command("npm run dev", "/work")]),
        Verdict::Allowed(Decision::Grant)
    );
    assert_eq!(
        inputs.decide(&[command("cargo watch -x test", "/work")]),
        Verdict::Allowed(Decision::Rule)
    );
}

#[test]
fn a_grant_or_a_rule_from_the_background_covers_both_modes() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![background_grant(command_grant("npm run dev", "/work"))];
    let mut background_rule = rule(&["cargo", "watch"], None);
    background_rule.background = true;
    inputs.rules = vec![background_rule];

    for access in [
        command("npm run dev", "/work"),
        in_background(command("npm run dev", "/work")),
    ] {
        assert_eq!(inputs.decide(&[access]), Verdict::Allowed(Decision::Grant));
    }
    for access in [
        command("cargo watch -x test", "/work"),
        in_background(command("cargo watch -x test", "/work")),
    ] {
        assert_eq!(inputs.decide(&[access]), Verdict::Allowed(Decision::Rule));
    }
}

#[test]
fn a_background_grant_covers_the_foreground_grant_of_the_same_command_but_not_the_reverse() {
    let foreground = command_grant("npm run dev", "/work");
    let background = background_grant(command_grant("npm run dev", "/work"));

    assert!(super::scope_covers(&background, &foreground));
    assert!(!super::scope_covers(&foreground, &background));
}

#[test]
fn a_rule_saved_from_the_background_keeps_its_mode() {
    let proposal = RuleProposal {
        words: vec!["cargo".to_owned(), "watch".to_owned(), "-x".to_owned()],
        folder: at("/work"),
        remote: None,
        program: None,
        background: true,
        terminal: None,
    };

    assert_eq!(
        saved_rule(&proposal, 2, false),
        Ok(SavedRule::Local {
            words: vec!["cargo".to_owned(), "watch".to_owned()],
            folder: None,
            background: true,
        })
    );
}

#[test]
fn a_host_rule_of_a_worker_allows_its_commands_in_the_background_too() {
    let inputs = Inputs::worker(CeilingMode::Autonomous);

    assert_eq!(
        inputs.decide(&[in_background(command("cargo test --workspace", "/srv/app"))]),
        Verdict::Allowed(Decision::HostRule)
    );
}

#[test]
fn saved_rules_keep_the_chosen_leading_words() {
    let proposal = RuleProposal {
        words: vec!["git".to_owned(), "status".to_owned(), "--short".to_owned()],
        folder: at("/work"),
        remote: None,
        program: None,
        background: false,
        terminal: None,
    };

    assert_eq!(
        saved_rule(&proposal, 2, false),
        Ok(SavedRule::Local {
            words: vec!["git".to_owned(), "status".to_owned()],
            folder: None,
            background: false,
        })
    );
    assert_eq!(
        saved_rule(&proposal, 3, true),
        Ok(SavedRule::Local {
            words: proposal.words.clone(),
            folder: Some(at("/work")),
            background: false,
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
        background: false,
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
        background: false,
    }
}

fn remote_rule(target: &str, words: &[&str]) -> RemoteCommandRule {
    RemoteCommandRule {
        id: RuleId::parse("0199e0f2-0000-7000-9000-000000000002").unwrap(),
        target: host(target, ENDPOINT),
        words: words.iter().map(|word| (*word).to_owned()).collect(),
        created_at: UtcMillis::EPOCH,
        background: false,
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
                program: None,
                background: false,
                terminal: None,
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
fn a_remote_grant_or_rule_from_the_foreground_still_asks_before_the_background() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![remote_grant("journalctl -f")];
    inputs.remote_rules = vec![remote_rule(SSH_HOST, &["tail", "-f"])];

    assert!(matches!(
        inputs.decide(&[in_background(remote("journalctl -f"))]),
        Verdict::Ask(_)
    ));
    assert!(matches!(
        inputs.decide(&[in_background(remote("tail -f /var/log/syslog"))]),
        Verdict::Ask(_)
    ));

    inputs.grants = vec![background_grant(remote_grant("journalctl -f"))];
    let mut background_rule = remote_rule(SSH_HOST, &["tail", "-f"]);
    background_rule.background = true;
    inputs.remote_rules = vec![background_rule];

    assert_eq!(
        inputs.decide(&[in_background(remote("journalctl -f"))]),
        Verdict::Allowed(Decision::Grant)
    );
    assert_eq!(
        inputs.decide(&[remote("journalctl -f")]),
        Verdict::Allowed(Decision::Grant)
    );
    assert_eq!(
        inputs.decide(&[in_background(remote("tail -f /var/log/syslog"))]),
        Verdict::Allowed(Decision::Rule)
    );
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
        program: None,
        background: false,
        terminal: None,
    };

    for limit_to_folder in [false, true] {
        assert_eq!(
            saved_rule(&proposal, 2, limit_to_folder),
            Ok(SavedRule::Remote {
                target: host(SSH_HOST, ENDPOINT),
                words: vec!["systemctl".to_owned(), "status".to_owned()],
                background: false,
            })
        );
    }
    assert_eq!(
        saved_rule(&proposal, 0, false),
        Err(ToolsError::RulePrefixInvalid)
    );
}

// Runs of installed programs (ADR-0038): never allowed by the workspace; a
// grant covers the same run with effects among its own, and a rule leading
// words of the same program with the effects of the run that it was saved
// from.

fn program_run_in(directory: &str, arguments: &[&str], effects: Vec<ProgramEffect>) -> Access {
    Access::Program {
        plugin: "git".to_owned(),
        name: "git".to_owned(),
        program: at("/usr/bin/git"),
        fixed: vec!["-c".to_owned(), "gc.auto=0".to_owned()],
        arguments: arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect(),
        working_directory: at(directory),
        effects,
        state: Some("HEAD 1a2b3c".to_owned()),
    }
}

fn program_run(arguments: &[&str], effects: Vec<ProgramEffect>) -> Access {
    program_run_in("/work", arguments, effects)
}

fn hook(name: &str) -> ProgramEffect {
    ProgramEffect::RunsFolderCode {
        name: name.to_owned(),
        source: format!(".git/hooks/{name}"),
    }
}

fn network(endpoint: &str) -> ProgramEffect {
    ProgramEffect::Network {
        endpoint: endpoint.to_owned(),
    }
}

fn discards(work: DiscardedWork, items: &[&str]) -> ProgramEffect {
    ProgramEffect::DiscardsWork {
        work,
        count: u32::try_from(items.len()).unwrap(),
        items: items.iter().map(|item| (*item).to_owned()).collect(),
        snapshot: true,
    }
}

const ORIGIN: &str = "https://example.com/repo.git";

fn git_target(effects: &[ProgramEffect]) -> ProgramTarget {
    ProgramTarget {
        plugin: "git".to_owned(),
        name: "git".to_owned(),
        program: at("/usr/bin/git"),
        effects: effect_keys(effects),
    }
}

/// The grant that "Allow for" gives a run.
fn program_grant(arguments: &[&str], effects: &[ProgramEffect]) -> GrantScope {
    GrantScope::Program {
        plugin: "git".to_owned(),
        name: "git".to_owned(),
        program: at("/usr/bin/git"),
        fixed: vec!["-c".to_owned(), "gc.auto=0".to_owned()],
        arguments: arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect(),
        working_directory: at("/work"),
        effects: effect_keys(effects),
    }
}

fn program_rule(words: &[&str], folder: Option<&str>, effects: &[ProgramEffect]) -> ProgramRule {
    ProgramRule {
        id: RuleId::parse("0199e0f2-0000-7000-9000-000000000003").unwrap(),
        target: git_target(effects),
        words: words.iter().map(|word| (*word).to_owned()).collect(),
        folder: folder.map(at),
        created_at: UtcMillis::EPOCH,
    }
}

#[test]
fn a_program_run_asks_beside_its_paths_and_offers_a_grant_and_a_rule() {
    let mut inputs = Inputs::new();
    inputs.allow_changes_in_workspace = true;
    let commit = ["commit", "-F", "-"];
    let effects = vec![hook("pre-commit")];
    let accesses = [
        path_access("/work", AccessMode::Write, true),
        program_run(&commit, effects.clone()),
    ];
    let proposal = RuleProposal {
        words: vec![
            "git".to_owned(),
            "commit".to_owned(),
            "-F".to_owned(),
            "-".to_owned(),
        ],
        folder: at("/work"),
        remote: None,
        program: Some(git_target(&effects)),
        background: false,
        terminal: None,
    };

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::Program],
            vec![program_grant(&commit, &effects)],
            Some(proposal.clone())
        )
    );
    // Without the setting, the change of the workspace asks too.
    inputs.allow_changes_in_workspace = false;
    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::ChangeInWorkspace, ApprovalReason::Program],
            vec![
                path_grant(AccessMode::Write, "/work"),
                program_grant(&commit, &effects)
            ],
            Some(proposal)
        )
    );
}

#[test]
fn neither_the_workspace_nor_a_commands_rule_or_grant_allows_a_program_run() {
    let mut inputs = Inputs::new();
    inputs.allow_changes_in_workspace = true;
    inputs.rules = vec![rule(&["git"], None)];
    inputs.grants = vec![command_grant("git commit -F -", "/work")];

    assert!(matches!(
        inputs.decide(&[program_run(&["commit", "-F", "-"], Vec::new())]),
        Verdict::Ask(_)
    ));
}

#[test]
fn a_grant_covers_the_same_run_with_effects_among_its_own() {
    let mut inputs = Inputs::new();
    let commit = ["commit", "-F", "-"];
    let hooks = [hook("pre-commit"), hook("commit-msg")];
    inputs.grants = vec![program_grant(&commit, &hooks)];

    for effects in [hooks.to_vec(), vec![hook("pre-commit")], Vec::new()] {
        assert_eq!(
            inputs.decide(&[program_run(&commit, effects.clone())]),
            Verdict::Allowed(Decision::Grant),
            "{effects:?}"
        );
    }
    // The state that the plugin read is for the digest alone.
    let mut later = program_run(&commit, hooks.to_vec());
    if let Access::Program { state, .. } = &mut later {
        *state = Some("HEAD 4d5e6f".to_owned());
    }
    assert_eq!(inputs.decide(&[later]), Verdict::Allowed(Decision::Grant));

    let mut other_plugin = program_run(&commit, Vec::new());
    if let Access::Program { plugin, .. } = &mut other_plugin {
        "tools".clone_into(plugin);
    }
    let mut other_program = program_run(&commit, Vec::new());
    if let Access::Program { program, .. } = &mut other_program {
        *program = at("/work/git");
    }
    let mut other_fixed = program_run(&commit, Vec::new());
    if let Access::Program { fixed, .. } = &mut other_fixed {
        fixed.clear();
    }
    for access in [
        program_run(&commit, vec![hook("pre-commit"), hook("post-commit")]),
        program_run(&["commit", "--amend", "-F", "-"], Vec::new()),
        program_run_in("/work/other", &commit, Vec::new()),
        program_run(&commit, vec![network(ORIGIN)]),
        other_plugin,
        other_program,
        other_fixed,
    ] {
        assert!(
            matches!(
                inputs.decide(std::slice::from_ref(&access)),
                Verdict::Ask(_)
            ),
            "{access:?}"
        );
    }

    let mut shared = Inputs::new();
    shared.project_grants = vec![program_grant(&commit, &hooks)];
    assert_eq!(
        shared.decide(&[program_run(&commit, hooks.to_vec())]),
        Verdict::Allowed(Decision::ProjectGrant)
    );
    shared.project_grants = Vec::new();
    shared.always_grants = vec![program_grant(&commit, &hooks)];
    assert_eq!(
        shared.decide(&[program_run(&commit, hooks.to_vec())]),
        Verdict::Allowed(Decision::AlwaysGrant)
    );
}

#[test]
fn a_grant_counts_discarded_work_by_its_kind() {
    let mut inputs = Inputs::new();
    let reset = ["reset", "--hard", "HEAD"];
    inputs.grants = vec![program_grant(
        &reset,
        &[discards(DiscardedWork::UncommittedChanges, &["a.txt"])],
    )];

    assert_eq!(
        inputs.decide(&[program_run(
            &reset,
            vec![discards(
                DiscardedWork::UncommittedChanges,
                &["b.txt", "c.txt"]
            )]
        )]),
        Verdict::Allowed(Decision::Grant)
    );
    assert!(matches!(
        inputs.decide(&[program_run(
            &reset,
            vec![discards(DiscardedWork::Commits, &["1a2b3c"])]
        )]),
        Verdict::Ask(_)
    ));
}

#[test]
fn a_program_rule_allows_leading_words_with_the_effects_that_it_covers() {
    let mut inputs = Inputs::new();
    inputs.program_rules = vec![program_rule(&["git", "push"], None, &[network(ORIGIN)])];
    let push = ["push", "origin", "main"];

    assert_eq!(
        inputs.decide(&[program_run(&push, vec![network(ORIGIN)])]),
        Verdict::Allowed(Decision::Rule)
    );
    for access in [
        program_run(&push, vec![network("https://example.com/other.git")]),
        program_run(&push, vec![network(ORIGIN), hook("pre-push")]),
        program_run(&["fetch", "origin"], vec![network(ORIGIN)]),
    ] {
        assert!(
            matches!(
                inputs.decide(std::slice::from_ref(&access)),
                Verdict::Ask(_)
            ),
            "{access:?}"
        );
    }
    // A rule of a program never matches a command.
    assert!(matches!(
        inputs.decide(&[command("git push origin main", "/work")]),
        Verdict::Ask(_)
    ));

    let mut limited = Inputs::new();
    limited.program_rules = vec![program_rule(&["git", "commit"], Some("/work"), &[])];
    assert_eq!(
        limited.decide(&[program_run_in("/work/sub", &["commit"], Vec::new())]),
        Verdict::Allowed(Decision::Rule)
    );
    assert!(matches!(
        limited.decide(&[program_run_in("/elsewhere", &["commit"], Vec::new())]),
        Verdict::Ask(_)
    ));
}

#[test]
fn a_program_run_in_mannies_folders_asks_without_a_grant_or_a_rule() {
    let mut inputs = Inputs::new();
    inputs.program_rules = vec![program_rule(&["git"], None, &[])];
    let accesses = [program_run_in("/data/mannie", &["status"], Vec::new())];

    assert_eq!(
        inputs.decide(&accesses),
        ask(&accesses, &[ApprovalReason::MannieFolder], Vec::new(), None)
    );
}

#[test]
fn two_program_runs_offer_a_grant_for_each_and_no_rule() {
    let inputs = Inputs::new();
    let accesses = [
        program_run(&["push", "origin", "main"], vec![network(ORIGIN)]),
        program_run(&["push", "origin", "v1"], vec![network(ORIGIN)]),
    ];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::Program],
            vec![
                program_grant(&["push", "origin", "main"], &[network(ORIGIN)]),
                program_grant(&["push", "origin", "v1"], &[network(ORIGIN)]),
            ],
            None
        )
    );
}

#[test]
fn full_access_allows_a_program_run() {
    let mut inputs = Inputs::new();
    inputs.full_access = true;

    assert_eq!(
        inputs.decide(&[program_run(
            &["push", "--force-with-lease", "origin", "main"],
            vec![network(ORIGIN), hook("pre-push")]
        )]),
        Verdict::Allowed(Decision::FullAccess)
    );
}

#[test]
fn a_worker_refuses_a_program_run_in_every_mode() {
    for mode in MODES.into_iter().chain([CeilingMode::FullAccess]) {
        assert_eq!(
            Inputs::worker(mode).decide(&[program_run_in("/srv/app", &["status"], Vec::new())]),
            Verdict::Refused(Refusal::NoProgramsOnHost),
            "{mode:?}"
        );
    }
}

#[test]
fn a_saved_program_rule_keeps_its_program_and_effects_and_may_be_limited_to_its_folder() {
    let effects = [hook("pre-commit")];
    let proposal = RuleProposal {
        words: vec!["git".to_owned(), "commit".to_owned(), "-F".to_owned()],
        folder: at("/work"),
        remote: None,
        program: Some(git_target(&effects)),
        background: false,
        terminal: None,
    };

    for (limit_to_folder, folder) in [(false, None), (true, Some(at("/work")))] {
        assert_eq!(
            saved_rule(&proposal, 2, limit_to_folder),
            Ok(SavedRule::Program {
                target: git_target(&effects),
                words: vec!["git".to_owned(), "commit".to_owned()],
                folder,
            })
        );
    }
    assert_eq!(
        saved_rule(&proposal, 4, false),
        Err(ToolsError::RulePrefixInvalid)
    );
}

// Calls of a tool server's tools (ADR-0043, decision 4): never allowed by the
// workspace; a grant covers the same tool of the same server at the same
// endpoint with any arguments, and no rule is offered.

const SERVER: &str = "0199e0f2-0000-7000-8000-0000000000cc";
const OTHER_SERVER: &str = "0199e0f2-0000-7000-8000-0000000000dd";
const SERVER_URL: &str = "https://mcp.example.com/mcp";

fn server_url(url: &str) -> ToolServerEndpoint {
    ToolServerEndpoint::Url {
        url: url.to_owned(),
    }
}

fn tool_call_on(target: &str, endpoint: ToolServerEndpoint, tool: &str, arguments: &str) -> Access {
    Access::ExternalTool {
        plugin: "mcp".to_owned(),
        target: target.to_owned(),
        label: "Issues".to_owned(),
        endpoint,
        tool: tool.to_owned(),
        title: Some("Create an issue".to_owned()),
        arguments: arguments.to_owned(),
        arguments_cut: false,
        hints: vec![ToolHint::OpenWorld],
    }
}

fn tool_call(tool: &str, arguments: &str) -> Access {
    tool_call_on(SERVER, server_url(SERVER_URL), tool, arguments)
}

fn tool_grant(tool: &str) -> GrantScope {
    GrantScope::ExternalTool {
        plugin: "mcp".to_owned(),
        target: SERVER.to_owned(),
        label: "Issues".to_owned(),
        endpoint: server_url(SERVER_URL),
        tool: tool.to_owned(),
    }
}

#[test]
fn a_tool_servers_call_asks_and_offers_a_grant_for_its_tool_and_no_rule() {
    let mut inputs = Inputs::new();
    inputs.allow_changes_in_workspace = true;
    let accesses = [tool_call("create_issue", r#"{"title":"Crash"}"#)];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::ExternalTool],
            vec![tool_grant("create_issue")],
            None
        )
    );
}

#[test]
fn a_grant_covers_the_same_tool_of_the_same_server_and_endpoint_with_any_arguments() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![tool_grant("create_issue")];
    let program = ToolServerEndpoint::Program {
        program: "npx".to_owned(),
        arguments: vec!["-y".to_owned(), "issues-server".to_owned()],
    };

    assert_eq!(
        inputs.decide(&[tool_call("create_issue", r#"{"title":"Another"}"#)]),
        Verdict::Allowed(Decision::Grant)
    );
    let mut other_plugin = tool_call("create_issue", "{}");
    if let Access::ExternalTool { plugin, .. } = &mut other_plugin {
        "other".clone_into(plugin);
    }
    for access in [
        tool_call("delete_issue", "{}"),
        tool_call_on(OTHER_SERVER, server_url(SERVER_URL), "create_issue", "{}"),
        tool_call_on(
            SERVER,
            server_url("https://mcp.example.com/v2"),
            "create_issue",
            "{}",
        ),
        tool_call_on(SERVER, program, "create_issue", "{}"),
        other_plugin,
    ] {
        assert!(
            matches!(
                inputs.decide(std::slice::from_ref(&access)),
                Verdict::Ask(_)
            ),
            "{access:?}"
        );
    }

    // The server's name, the tool's title and the server's hints are for
    // display: they change nothing that a grant covers.
    let mut renamed = tool_call("create_issue", "{}");
    if let Access::ExternalTool {
        label,
        title,
        hints,
        ..
    } = &mut renamed
    {
        "Tracker".clone_into(label);
        *title = None;
        *hints = vec![ToolHint::ReadOnly];
    }
    assert_eq!(inputs.decide(&[renamed]), Verdict::Allowed(Decision::Grant));

    let mut shared = Inputs::new();
    shared.project_grants = vec![tool_grant("create_issue")];
    assert_eq!(
        shared.decide(&[tool_call("create_issue", "{}")]),
        Verdict::Allowed(Decision::ProjectGrant)
    );
    shared.project_grants = Vec::new();
    shared.always_grants = vec![tool_grant("create_issue")];
    assert_eq!(
        shared.decide(&[tool_call("create_issue", "{}")]),
        Verdict::Allowed(Decision::AlwaysGrant)
    );
}

#[test]
fn neither_the_workspace_nor_another_kind_of_grant_allows_a_tool_servers_call() {
    let mut inputs = Inputs::new();
    inputs.allow_changes_in_workspace = true;
    inputs.rules = vec![rule(&["create_issue"], None)];
    inputs.grants = vec![
        path_grant(AccessMode::Write, "/work"),
        command_grant("create_issue", "/work"),
        remote_grant("create_issue"),
    ];

    assert!(matches!(
        inputs.decide(&[tool_call("create_issue", "{}")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn full_access_allows_a_tool_servers_call() {
    let mut inputs = Inputs::new();
    inputs.full_access = true;

    assert_eq!(
        inputs.decide(&[tool_call("delete_repository", r#"{"name":"app"}"#)]),
        Verdict::Allowed(Decision::FullAccess)
    );
}

#[test]
fn a_worker_refuses_a_tool_servers_call_in_every_mode() {
    for mode in MODES.into_iter().chain([CeilingMode::FullAccess]) {
        assert_eq!(
            Inputs::worker(mode).decide(&[tool_call("create_issue", "{}")]),
            Verdict::Refused(Refusal::NoExternalTools),
            "{mode:?}"
        );
    }
}

#[test]
fn a_tool_grant_covers_another_only_for_the_same_tool_of_the_same_server() {
    let mut renamed = tool_grant("create_issue");
    if let GrantScope::ExternalTool { label, .. } = &mut renamed {
        "Tracker".clone_into(label);
    }

    assert!(super::scope_covers(&tool_grant("create_issue"), &renamed));
    assert!(!super::scope_covers(
        &tool_grant("create_issue"),
        &tool_grant("delete_issue")
    ));
    assert!(!super::scope_covers(
        &tool_grant("create_issue"),
        &remote_grant("create_issue")
    ));
}

/// A command that the terminal plugin types into terminal 2 of a host
/// (ADR-0053, decision 5).
fn typed_on(host: TerminalHost, text: &str) -> Access {
    Access::TerminalCommand {
        plugin: "terminal".to_owned(),
        host,
        identity: String::new(),
        terminal: Some(2),
        shell: "bash".to_owned(),
        state: Some("owner in /work".to_owned()),
        command: text.to_owned(),
    }
}

fn this_machine(shell: &str) -> TerminalHost {
    TerminalHost::ThisMachine {
        shell: shell.to_owned(),
    }
}

fn ssh_host(endpoint: &str) -> TerminalHost {
    TerminalHost::Remote {
        plugin: "ssh".to_owned(),
        target: "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
        label: "build-box".to_owned(),
        endpoint: endpoint.to_owned(),
    }
}

fn typed(text: &str) -> Access {
    typed_on(this_machine("/bin/bash"), text)
}

fn terminal_grant(host: TerminalHost, text: &str) -> GrantScope {
    GrantScope::TerminalCommand {
        plugin: "terminal".to_owned(),
        host,
        command: text.to_owned(),
    }
}

fn terminal_rule(host: TerminalHost, words: &[&str]) -> TerminalCommandRule {
    TerminalCommandRule {
        id: RuleId::parse("0199e0f2-0000-7000-9000-000000000009").unwrap(),
        target: TerminalTarget {
            plugin: "terminal".to_owned(),
            host,
        },
        words: words.iter().map(|word| (*word).to_owned()).collect(),
        created_at: UtcMillis::EPOCH,
    }
}

#[test]
fn a_terminal_command_asks_and_offers_a_grant_and_a_rule_for_its_host() {
    let accesses = [typed("ss -ltnp")];

    assert_eq!(
        Inputs::new().decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::TerminalCommand],
            vec![terminal_grant(this_machine("/bin/bash"), "ss -ltnp")],
            Some(RuleProposal {
                words: vec!["ss".to_owned(), "-ltnp".to_owned()],
                folder: String::new(),
                remote: None,
                program: None,
                background: false,
                terminal: Some(TerminalTarget {
                    plugin: "terminal".to_owned(),
                    host: this_machine("/bin/bash"),
                }),
            }),
        )
    );
}

#[test]
fn the_workspace_never_allows_a_terminal_command() {
    let mut inputs = Inputs::new();
    inputs.allow_changes_in_workspace = true;

    assert!(matches!(inputs.decide(&[typed("ls")]), Verdict::Ask(_)));
}

#[test]
fn a_terminal_grant_covers_any_terminal_of_its_host_and_shell_only() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![terminal_grant(this_machine("/bin/bash"), "ss -ltnp")];
    let mut other_terminal = typed("ss -ltnp");
    if let Access::TerminalCommand {
        terminal, state, ..
    } = &mut other_terminal
    {
        *terminal = None;
        *state = None;
    }

    assert_eq!(
        inputs.decide(&[typed("ss -ltnp")]),
        Verdict::Allowed(Decision::Grant)
    );
    assert_eq!(
        inputs.decide(&[other_terminal]),
        Verdict::Allowed(Decision::Grant)
    );
    assert!(matches!(
        inputs.decide(&[typed_on(this_machine("/bin/zsh"), "ss -ltnp")]),
        Verdict::Ask(_)
    ));
    assert!(matches!(
        inputs.decide(&[typed("ss -ltn")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn terminal_grants_and_the_grants_of_other_commands_never_cover_each_other() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![command_grant("uname -a", "/work"), remote_grant("uname -a")];
    assert!(matches!(
        inputs.decide(&[typed("uname -a")]),
        Verdict::Ask(_)
    ));

    let mut inputs = Inputs::new();
    inputs.grants = vec![terminal_grant(this_machine("/bin/bash"), "uname -a")];
    assert!(matches!(
        inputs.decide(&[command("uname -a", "/work")]),
        Verdict::Ask(_)
    ));
    assert!(matches!(
        inputs.decide(&[remote("uname -a")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn a_terminal_grant_on_a_remote_host_holds_through_a_rename_and_not_on_another_endpoint() {
    let mut inputs = Inputs::new();
    inputs.always_grants = vec![terminal_grant(ssh_host("deploy@build-box:22"), "uptime")];
    let mut renamed = ssh_host("deploy@build-box:22");
    if let TerminalHost::Remote { label, .. } = &mut renamed {
        "renamed".clone_into(label);
    }

    assert_eq!(
        inputs.decide(&[typed_on(renamed, "uptime")]),
        Verdict::Allowed(Decision::AlwaysGrant)
    );
    assert!(matches!(
        inputs.decide(&[typed_on(ssh_host("root@build-box:22"), "uptime")]),
        Verdict::Ask(_)
    ));
}

#[test]
fn a_terminal_rule_is_limited_to_its_plugin_and_host() {
    let mut inputs = Inputs::new();
    inputs.terminal_rules = vec![terminal_rule(
        ssh_host("deploy@build-box:22"),
        &["git", "status"],
    )];

    assert_eq!(
        inputs.decide(&[typed_on(
            ssh_host("deploy@build-box:22"),
            "git status --short"
        )]),
        Verdict::Allowed(Decision::Rule)
    );
    assert!(matches!(
        inputs.decide(&[typed_on(ssh_host("deploy@other:22"), "git status")]),
        Verdict::Ask(_)
    ));
    assert!(matches!(
        inputs.decide(&[typed("git status")]),
        Verdict::Ask(_)
    ));
    let mut other_plugin = typed_on(ssh_host("deploy@build-box:22"), "git status");
    if let Access::TerminalCommand { plugin, .. } = &mut other_plugin {
        "other".clone_into(plugin);
    }
    assert!(matches!(inputs.decide(&[other_plugin]), Verdict::Ask(_)));
}

#[test]
fn a_terminal_command_with_metacharacters_can_only_be_allowed_once() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![terminal_grant(this_machine("/bin/bash"), "ls | wc -l")];
    inputs.terminal_rules = vec![terminal_rule(this_machine("/bin/bash"), &["ls"])];
    let accesses = [typed("ls | wc -l")];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::TerminalCommand],
            Vec::new(),
            None
        )
    );
}

#[test]
fn full_access_allows_a_terminal_command_and_a_worker_refuses_it() {
    let mut inputs = Inputs::new();
    inputs.full_access = true;
    assert_eq!(
        inputs.decide(&[typed("rm -rf build")]),
        Verdict::Allowed(Decision::FullAccess)
    );

    for mode in MODES.into_iter().chain([CeilingMode::FullAccess]) {
        assert_eq!(
            Inputs::worker(mode).decide(&[typed("ls")]),
            Verdict::Refused(Refusal::NoTerminals),
            "{mode:?}"
        );
    }
}

#[test]
fn a_saved_terminal_rule_keeps_its_host_and_never_a_folder() {
    let proposal = RuleProposal {
        words: vec!["git".to_owned(), "status".to_owned(), "--short".to_owned()],
        folder: String::new(),
        remote: None,
        program: None,
        background: false,
        terminal: Some(TerminalTarget {
            plugin: "terminal".to_owned(),
            host: ssh_host("deploy@build-box:22"),
        }),
    };

    assert_eq!(
        saved_rule(&proposal, 2, true),
        Ok(SavedRule::Terminal {
            target: TerminalTarget {
                plugin: "terminal".to_owned(),
                host: ssh_host("deploy@build-box:22"),
            },
            words: vec!["git".to_owned(), "status".to_owned()],
        })
    );
}

#[test]
fn a_terminal_grant_covers_another_only_on_the_same_host_and_command() {
    assert!(super::scope_covers(
        &terminal_grant(ssh_host("deploy@build-box:22"), "uptime"),
        &terminal_grant(ssh_host("deploy@build-box:22"), "uptime")
    ));
    assert!(!super::scope_covers(
        &terminal_grant(ssh_host("deploy@build-box:22"), "uptime"),
        &terminal_grant(ssh_host("deploy@other:22"), "uptime")
    ));
    assert!(!super::scope_covers(
        &terminal_grant(this_machine("/bin/bash"), "uptime"),
        &command_grant("uptime", "/work")
    ));
}

const DATABASE: &str = "0199e0f2-0000-7000-8000-0000000000ee";
const OTHER_DATABASE: &str = "0199e0f2-0000-7000-8000-0000000000ef";
const UPDATE: &str = "UPDATE parts SET stock = ?1 WHERE name = ?2";
const CREATE_INDEX: &str = "CREATE INDEX parts_name ON parts(name)";

/// A statement on a database that the owner registered for the plugin
/// `workers`, of the class that SQLite reported (ADR-0056, decision 7).
fn statement_on(key: &str, text: &str, parameters: &[&str], class: StatementClass) -> Access {
    let parameters: Vec<String> = parameters.iter().map(|value| (*value).to_owned()).collect();
    Access::Database {
        plugin: "workers".to_owned(),
        key: DatabaseKey::parse(key).unwrap(),
        name: "Inventory".to_owned(),
        home: DatabaseHome::ThisComputer,
        identity: "7:42".to_owned(),
        statement: text.to_owned(),
        statement_cut: false,
        parameters_digest: parameters.join(","),
        parameter_count: u32::try_from(parameters.len()).unwrap(),
        parameters,
        class,
    }
}

fn statement(text: &str, parameters: &[&str], class: StatementClass) -> Access {
    statement_on(DATABASE, text, parameters, class)
}

/// A statement on a database of the worker `build-box`, whose host is in
/// `mode` and takes the master's full access when `accepts` says so
/// (ADR-0057, decision 6).
fn on_worker(text: &str, class: StatementClass, mode: CeilingMode, accepts: bool) -> Access {
    let mut access = statement(text, &["12"], class);
    if let Access::Database { home, .. } = &mut access {
        *home = DatabaseHome::Worker {
            worker: WorkerId::parse("0199e0f2-0000-7000-8000-0000000000aa").unwrap(),
            name: "build-box".to_owned(),
            mode,
            accepts_full_access: accepts,
        };
    }
    access
}

fn database_grant(text: &str) -> GrantScope {
    GrantScope::Database {
        plugin: "workers".to_owned(),
        key: DatabaseKey::parse(DATABASE).unwrap(),
        name: "Inventory".to_owned(),
        statement: text.to_owned(),
    }
}

#[test]
fn a_read_of_a_registered_database_runs_in_every_conversation_and_names_its_source() {
    let mut without_workspace = Inputs::new();
    without_workspace.workspace = None;
    let query = statement(
        "SELECT * FROM parts WHERE name = ?1",
        &["'bolt'"],
        StatementClass::Read,
    );

    for inputs in [Inputs::new(), without_workspace] {
        assert_eq!(
            inputs.decide(std::slice::from_ref(&query)),
            Verdict::Allowed(Decision::RegisteredDatabase)
        );
    }
    assert_eq!(
        Inputs::new().decide(&[read("/work/a.txt"), query.clone()]),
        Verdict::Allowed(Decision::RegisteredDatabase)
    );
    let mut granted = Inputs::new();
    granted.grants = vec![path_grant(AccessMode::Read, "/other")];
    assert_eq!(
        granted.decide(&[read("/other/b.txt"), query]),
        Verdict::Allowed(Decision::Grant)
    );
}

#[test]
fn a_database_write_asks_and_offers_a_grant_of_its_statement_and_no_rule() {
    let mut inputs = Inputs::new();
    inputs.allow_changes_in_workspace = true;
    let accesses = [statement(UPDATE, &["12", "'bolt'"], StatementClass::Write)];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::DatabaseWrite],
            vec![database_grant(UPDATE)],
            None
        )
    );
    let twice = [
        statement(UPDATE, &["12", "'bolt'"], StatementClass::Write),
        statement(UPDATE, &["4", "'nut'"], StatementClass::Write),
    ];
    assert_eq!(
        inputs.decide(&twice),
        ask(
            &twice,
            &[ApprovalReason::DatabaseWrite],
            vec![database_grant(UPDATE)],
            None
        )
    );
}

#[test]
fn a_database_grant_covers_the_same_statement_on_the_same_database_with_any_parameters() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![database_grant(UPDATE)];
    let mut renamed = statement(UPDATE, &["3", "'nut'"], StatementClass::Write);
    if let Access::Database { name, .. } = &mut renamed {
        "Stock".clone_into(name);
    }
    let mut other_plugin = statement(UPDATE, &["12", "'bolt'"], StatementClass::Write);
    if let Access::Database { plugin, .. } = &mut other_plugin {
        "other".clone_into(plugin);
    }

    assert_eq!(
        inputs.decide(&[statement(UPDATE, &["3", "'nut'"], StatementClass::Write)]),
        Verdict::Allowed(Decision::Grant)
    );
    // The database's name is for display: a rename changes nothing that a
    // grant covers.
    assert_eq!(inputs.decide(&[renamed]), Verdict::Allowed(Decision::Grant));
    for access in [
        statement(
            "UPDATE parts SET stock = ?1",
            &["12"],
            StatementClass::Write,
        ),
        statement_on(
            OTHER_DATABASE,
            UPDATE,
            &["12", "'bolt'"],
            StatementClass::Write,
        ),
        other_plugin,
    ] {
        assert!(
            matches!(
                inputs.decide(std::slice::from_ref(&access)),
                Verdict::Ask(_)
            ),
            "{access:?}"
        );
    }

    let mut shared = Inputs::new();
    shared.project_grants = vec![database_grant(UPDATE)];
    assert_eq!(
        shared.decide(&[statement(UPDATE, &[], StatementClass::Write)]),
        Verdict::Allowed(Decision::ProjectGrant)
    );
    shared.project_grants = Vec::new();
    shared.always_grants = vec![database_grant(UPDATE)];
    assert_eq!(
        shared.decide(&[statement(UPDATE, &[], StatementClass::Write)]),
        Verdict::Allowed(Decision::AlwaysGrant)
    );
}

#[test]
fn a_change_of_a_schema_always_asks_and_no_grant_covers_it() {
    let mut inputs = Inputs::new();
    inputs.allow_changes_in_workspace = true;
    inputs.grants = vec![database_grant(CREATE_INDEX)];
    inputs.always_grants = vec![database_grant(CREATE_INDEX)];
    let accesses = [statement(CREATE_INDEX, &[], StatementClass::Schema)];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::DatabaseSchema],
            Vec::new(),
            None
        )
    );
    // A grant of the write would not cover the call, so the approval offers
    // none for it.
    let both = [
        statement(UPDATE, &["1", "'bolt'"], StatementClass::Write),
        statement(CREATE_INDEX, &[], StatementClass::Schema),
    ];
    assert_eq!(
        Inputs::new().decide(&both),
        ask(
            &both,
            &[
                ApprovalReason::DatabaseWrite,
                ApprovalReason::DatabaseSchema
            ],
            Vec::new(),
            None
        )
    );
}

#[test]
fn neither_the_workspace_nor_another_kind_of_grant_or_rule_allows_a_database_write() {
    let mut inputs = Inputs::new();
    inputs.allow_changes_in_workspace = true;
    inputs.rules = vec![rule(&["UPDATE"], None)];
    inputs.grants = vec![
        path_grant(AccessMode::Write, "/work"),
        command_grant(UPDATE, "/work"),
        remote_grant(UPDATE),
        tool_grant(UPDATE),
        terminal_grant(this_machine("/bin/bash"), UPDATE),
    ];

    assert!(matches!(
        inputs.decide(&[statement(UPDATE, &[], StatementClass::Write)]),
        Verdict::Ask(_)
    ));
}

#[test]
fn an_approval_covers_the_values_and_the_file_that_it_showed() {
    let shown = [statement(UPDATE, &["12", "'bolt'"], StatementClass::Write)];
    let mut replaced = statement(UPDATE, &["12", "'bolt'"], StatementClass::Write);
    if let Access::Database { identity, .. } = &mut replaced {
        "7:43".clone_into(identity);
    }
    // The approval shows long values cut, so two values that show alike
    // still differ in the digest of the full values.
    let mut cut_alike = statement(UPDATE, &["12", "'bolt'"], StatementClass::Write);
    if let Access::Database {
        parameters_digest, ..
    } = &mut cut_alike
    {
        "another digest".clone_into(parameters_digest);
    }

    for other in [
        statement(UPDATE, &["13", "'bolt'"], StatementClass::Write),
        replaced,
        cut_alike,
    ] {
        assert_ne!(
            digest(&shown),
            digest(std::slice::from_ref(&other)),
            "{other:?}"
        );
    }
}

#[test]
fn a_read_of_a_workers_database_runs_whatever_its_hosts_mode() {
    for mode in MODES.into_iter().chain([CeilingMode::FullAccess]) {
        assert_eq!(
            Inputs::new().decide(&[on_worker("SELECT 1", StatementClass::Read, mode, false)]),
            Verdict::Allowed(Decision::RegisteredDatabase),
            "{mode:?}"
        );
    }
}

#[test]
fn a_read_only_hosts_database_refuses_every_write_even_in_full_access() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![database_grant(UPDATE)];
    for full_access in [false, true] {
        inputs.full_access = full_access;
        for class in [StatementClass::Write, StatementClass::Schema] {
            assert_eq!(
                inputs.decide(&[on_worker(UPDATE, class, CeilingMode::ReadOnly, true)]),
                Verdict::Refused(Refusal::ReadOnlyHost),
                "{full_access} {class:?}"
            );
        }
    }
}

#[test]
fn an_ask_hosts_write_offers_only_allow_once_and_no_grant_covers_it() {
    let mut inputs = Inputs::new();
    inputs.grants = vec![database_grant(UPDATE)];
    inputs.always_grants = vec![database_grant(UPDATE)];
    let accesses = [on_worker(
        UPDATE,
        StatementClass::Write,
        CeilingMode::Ask,
        true,
    )];

    assert_eq!(
        inputs.decide(&accesses),
        ask(
            &accesses,
            &[ApprovalReason::DatabaseWrite],
            Vec::new(),
            None
        )
    );
}

#[test]
fn an_autonomous_hosts_write_offers_a_grant_and_runs_under_one() {
    for mode in [CeilingMode::Autonomous, CeilingMode::FullAccess] {
        let accesses = [on_worker(UPDATE, StatementClass::Write, mode, false)];
        assert_eq!(
            Inputs::new().decide(&accesses),
            ask(
                &accesses,
                &[ApprovalReason::DatabaseWrite],
                vec![database_grant(UPDATE)],
                None
            ),
            "{mode:?}"
        );
        let mut granted = Inputs::new();
        granted.grants = vec![database_grant(UPDATE)];
        assert_eq!(
            granted.decide(&accesses),
            Verdict::Allowed(Decision::Grant),
            "{mode:?}"
        );
    }
    let schema = [on_worker(
        CREATE_INDEX,
        StatementClass::Schema,
        CeilingMode::Autonomous,
        false,
    )];
    assert_eq!(
        Inputs::new().decide(&schema),
        ask(&schema, &[ApprovalReason::DatabaseSchema], Vec::new(), None)
    );
}

#[test]
fn full_access_counts_for_a_workers_write_only_where_its_host_takes_it() {
    let mut inputs = Inputs::new();
    inputs.full_access = true;
    for (mode, accepts) in [
        (CeilingMode::Ask, true),
        (CeilingMode::Autonomous, true),
        (CeilingMode::FullAccess, false),
    ] {
        for class in [StatementClass::Write, StatementClass::Schema] {
            assert_eq!(
                inputs.decide(&[on_worker(UPDATE, class, mode, accepts)]),
                Verdict::Allowed(Decision::FullAccess),
                "{mode:?} {class:?}"
            );
        }
    }
    let refused = [on_worker(
        UPDATE,
        StatementClass::Write,
        CeilingMode::Ask,
        false,
    )];
    assert_eq!(
        inputs.decide(&refused),
        ask(&refused, &[ApprovalReason::DatabaseWrite], Vec::new(), None)
    );
    let schema = [on_worker(
        CREATE_INDEX,
        StatementClass::Schema,
        CeilingMode::Autonomous,
        false,
    )];
    assert_eq!(
        inputs.decide(&schema),
        ask(&schema, &[ApprovalReason::DatabaseSchema], Vec::new(), None)
    );
    // A grant still covers the write on an autonomous host that refuses the
    // master's full access, and full access covers the rest of the call.
    inputs.grants = vec![database_grant(UPDATE)];
    assert_eq!(
        inputs.decide(&[
            read("/elsewhere/a.txt"),
            on_worker(
                UPDATE,
                StatementClass::Write,
                CeilingMode::Autonomous,
                false
            ),
        ]),
        Verdict::Allowed(Decision::FullAccess)
    );
}

#[test]
fn full_access_allows_every_statement_and_a_worker_refuses_every_one() {
    let mut inputs = Inputs::new();
    inputs.full_access = true;
    for class in [
        StatementClass::Read,
        StatementClass::Write,
        StatementClass::Schema,
    ] {
        assert_eq!(
            inputs.decide(&[statement("DROP TABLE parts", &[], class)]),
            Verdict::Allowed(Decision::FullAccess),
            "{class:?}"
        );
    }

    for mode in MODES.into_iter().chain([CeilingMode::FullAccess]) {
        assert_eq!(
            Inputs::worker(mode).decide(&[statement("SELECT 1", &[], StatementClass::Read)]),
            Verdict::Refused(Refusal::NoDatabases),
            "{mode:?}"
        );
    }
}

#[test]
fn a_database_grant_covers_another_only_for_the_same_statement_on_the_same_database() {
    let mut renamed = database_grant(UPDATE);
    if let GrantScope::Database { name, .. } = &mut renamed {
        "Stock".clone_into(name);
    }
    let mut other_database = database_grant(UPDATE);
    if let GrantScope::Database { key, .. } = &mut other_database {
        *key = DatabaseKey::parse(OTHER_DATABASE).unwrap();
    }

    assert!(super::scope_covers(&database_grant(UPDATE), &renamed));
    assert!(!super::scope_covers(
        &database_grant(UPDATE),
        &database_grant("DELETE FROM parts")
    ));
    assert!(!super::scope_covers(
        &database_grant(UPDATE),
        &other_database
    ));
    assert!(!super::scope_covers(
        &database_grant(UPDATE),
        &command_grant(UPDATE, "/work")
    ));
}

/// The folder of the skill `pdf` of this computer.
const PDF_SKILL: &str = "/home/owner/.agents/skills/pdf";

#[test]
fn a_read_inside_a_skill_that_is_on_runs_without_asking_and_names_its_source() {
    let inputs = Inputs {
        skill_folders: vec![PathBuf::from(at(PDF_SKILL))],
        ..Inputs::new()
    };

    for path in [
        format!("{PDF_SKILL}/SKILL.md"),
        format!("{PDF_SKILL}/references/forms.md"),
    ] {
        assert_eq!(
            inputs.decide(&[read(&path)]),
            Verdict::Allowed(Decision::Skill),
            "{path}"
        );
    }
    // Another skill's folder, and one whose name only starts the same way,
    // still ask.
    for path in [
        "/home/owner/.agents/skills/docx/SKILL.md",
        "/home/owner/.agents/skills/pdf-tools/SKILL.md",
    ] {
        assert!(
            matches!(inputs.decide(&[read(path)]), Verdict::Ask(_)),
            "{path}"
        );
    }
}

#[test]
fn a_change_inside_a_skill_asks_as_any_other_change() {
    let inputs = Inputs {
        skill_folders: vec![PathBuf::from(at(PDF_SKILL))],
        ..Inputs::new()
    };

    let verdict = inputs.decide(&[write(&format!("{PDF_SKILL}/SKILL.md"))]);

    let Verdict::Ask(request) = verdict else {
        panic!("a change asks: {verdict:?}");
    };
    assert_eq!(request.reasons, [ApprovalReason::OutsideWorkspace]);
}

#[test]
fn a_skill_inside_mannie_s_own_folders_still_asks_without_a_grant() {
    let inputs = Inputs {
        skill_folders: vec![PathBuf::from(at("/data/mannie/skills/pdf"))],
        ..Inputs::new()
    };

    let verdict = inputs.decide(&[read("/data/mannie/skills/pdf/SKILL.md")]);

    let Verdict::Ask(request) = verdict else {
        panic!("Mannie's own folders ask: {verdict:?}");
    };
    assert_eq!(request.reasons, [ApprovalReason::MannieFolder]);
    assert_eq!(request.grants, Vec::<GrantScope>::new());
}

#[test]
fn a_read_of_a_project_s_skill_keeps_the_workspace_as_its_source() {
    let inputs = Inputs {
        skill_folders: vec![PathBuf::from(at("/work/.agents/skills/pdf"))],
        ..Inputs::new()
    };

    assert_eq!(
        inputs.decide(&[read("/work/.agents/skills/pdf/SKILL.md")]),
        Verdict::Allowed(Decision::Workspace)
    );
}
