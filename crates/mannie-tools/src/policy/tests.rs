use std::path::PathBuf;

use mannie_types::clock::UtcMillis;
use mannie_types::ids::{ProjectId, RuleId};
use mannie_types::tools::{
    Access, AccessMode, ApprovalReason, ApprovalRequest, CommandRule, Decision, EntryCount,
    GrantScope, RuleProposal, ToolsError,
};

use super::{Grants, PolicyInputs, Verdict, decide, saved_rule};
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
    project: Option<ProjectId>,
    owner_request: Option<String>,
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
            project: None,
            owner_request: None,
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
                project: self.project.as_ref(),
                owner_request: self.owner_request.as_deref(),
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
    Verdict::Ask(ApprovalRequest {
        digest: digest(accesses),
        reasons: reasons.to_vec(),
        grants,
        rule,
        project: None,
    })
}

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
    };

    assert_eq!(
        saved_rule(&proposal, 2, false),
        Ok((vec!["git".to_owned(), "status".to_owned()], None))
    );
    assert_eq!(
        saved_rule(&proposal, 3, true),
        Ok((proposal.words.clone(), Some(at("/work"))))
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
