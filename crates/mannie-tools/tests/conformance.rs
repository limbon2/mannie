//! The conformance suite of the system tools (docs/features/system-tools/
//! tools-and-results.md, acceptance): every tool against real temporary
//! folders, through its success paths and its failure codes.

#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mannie_store::{Store, StoreOptions};
#[cfg(any(windows, target_os = "linux"))]
use mannie_tools::StopReason;
use mannie_tools::{
    Availability, CommandEnvironment, Done, Execution, MoveFailure, MutationLock, PrepareError,
    Prepared, ProjectFacts, ProjectMoves, StopSignal, SystemFacts, ToolContext, ToolRegistry,
};
use mannie_types::clock::{Clock, SystemClock, UtcMillis};
use mannie_types::ids::{ArchiveId, ConversationId, ProjectId};
use mannie_types::projects::ProjectView;
#[cfg(any(windows, target_os = "linux"))]
use mannie_types::tools::CommandEnding;
use mannie_types::tools::{Access, AccessMode, EntryCount, EntryKind, ToolFailure, ToolOutcome};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A workspace in a temporary folder, a store and the system tools.
struct Harness {
    _folder: TempDir,
    root: PathBuf,
    context: ToolContext,
    registry: ToolRegistry,
}

impl Harness {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let workspace = folder.path().join("work");
        fs::create_dir(&workspace).unwrap();
        let root = PathBuf::from(mannie_tools::canonical_folder(&workspace).unwrap());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let options = StoreOptions::new(folder.path().join("backups"), clock);
        let store = Store::open(&folder.path().join("mannie.sqlite"), &options).unwrap();
        let context = ToolContext {
            conversation_id: ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap(),
            workspace: Some(root.clone()),
            store,
            result_limit: mannie_tools::RESULT_LIMIT_BYTES,
            environment: Arc::new(CommandEnvironment::from_process(&[])),
            mutations: MutationLock::new(),
            archive_ids: Arc::new(|| {
                ArchiveId::parse("0199e0f2-0000-7000-a000-000000000001").unwrap()
            }),
            system: Arc::new(SystemFacts::detect()),
            projects: ProjectFacts::none(),
        };
        Self {
            _folder: folder,
            root,
            context,
            registry: ToolRegistry::with_system_tools(),
        }
    }

    /// A path in the workspace.
    fn path(&self, relative: &str) -> PathBuf {
        let mut path = self.root.clone();
        for name in relative.split('/') {
            path.push(name);
        }
        path
    }

    /// The text of a path in the workspace.
    fn text(&self, relative: &str) -> String {
        self.path(relative).to_str().unwrap().to_owned()
    }

    fn file(&self, relative: &str, content: impl AsRef<[u8]>) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn folder(&self, relative: &str) {
        fs::create_dir_all(self.path(relative)).unwrap();
    }

    async fn prepare(&self, tool: &str, arguments: &Value) -> Result<Prepared, PrepareError> {
        let tool = self.registry.get(tool).unwrap();
        tool.prepare(self.context.clone(), arguments.to_string())
            .await
    }

    /// Executes a call with the accesses that were authorized.
    async fn execute(
        &self,
        tool: &str,
        arguments: &Value,
        accesses: Vec<Access>,
        stop: StopSignal,
    ) -> Done {
        let tool = self.registry.get(tool).unwrap();
        tool.execute(Execution {
            context: self.context.clone(),
            arguments: arguments.to_string(),
            accesses,
            stop,
            process: None,
        })
        .await
    }

    /// Prepares and executes a call, as the runtime does once the policy
    /// allowed it.
    async fn call(&self, tool: &str, arguments: Value) -> Result<Done, PrepareError> {
        let prepared = self.prepare(tool, &arguments).await?;
        Ok(self
            .execute(tool, &arguments, prepared.accesses, StopSignal::new())
            .await)
    }

    /// A call that must succeed.
    async fn done(&self, tool: &str, arguments: Value) -> Done {
        let done = self.call(tool, arguments).await.unwrap();
        assert!(
            !matches!(done.outcome, ToolOutcome::Failed { .. }),
            "{}",
            done.output
        );
        done
    }

    /// A call that must fail, at its preparation or its execution.
    async fn failure(&self, tool: &str, arguments: Value) -> ToolFailure {
        match self.call(tool, arguments).await {
            Err(PrepareError::Failed(failure))
            | Ok(Done {
                outcome: ToolOutcome::Failed { failure },
                ..
            }) => failure,
            other => panic!("expected a failure, got {other:?}"),
        }
    }
}

fn with_small_limit(mut harness: Harness, limit: usize) -> Harness {
    harness.context.result_limit = limit;
    harness
}

#[tokio::test]
async fn system_info_names_the_system_the_shell_and_the_workspace() {
    let harness = Harness::new();

    let done = harness.done("system_info", json!({})).await;

    assert!(
        done.output.contains("Operating system: "),
        "{}",
        done.output
    );
    assert!(
        done.output
            .contains(&format!("Workspace: {}", harness.root.display())),
        "{}",
        done.output
    );
    assert!(matches!(done.outcome, ToolOutcome::SystemInfo { .. }));
    // A model may send no arguments at all.
    assert!(
        harness
            .prepare("system_info", &Value::Null)
            .await
            .is_ok_and(|p| p.accesses.is_empty())
    );
}

#[tokio::test]
async fn unknown_arguments_are_refused_before_anything_runs() {
    let harness = Harness::new();

    let refused = harness
        .prepare("read_file", &json!({ "path": "a.txt", "follow": true }))
        .await;

    assert!(
        matches!(refused, Err(PrepareError::InvalidArguments { .. })),
        "{refused:?}"
    );
}

#[tokio::test]
async fn list_directory_lists_entries_sorted_by_name_in_pages() {
    let harness = Harness::new();
    harness.file("b.txt", "bee");
    harness.file("A.txt", "a");
    harness.folder("c");

    let done = harness.done("list_directory", json!({})).await;

    let lines: Vec<&str> = done.output.lines().collect();
    assert_eq!(lines[0], format!("Folder: {}", harness.root.display()));
    assert_eq!(lines[1], "Entries 1 to 3 of 3.");
    assert!(
        lines[2].starts_with("[file] A.txt (1 bytes, changed "),
        "{}",
        lines[2]
    );
    assert!(
        lines[3].starts_with("[file] b.txt (3 bytes, changed "),
        "{}",
        lines[3]
    );
    assert!(lines[4].starts_with("[folder] c ("), "{}", lines[4]);
    assert_eq!(
        done.outcome,
        ToolOutcome::Listed {
            path: harness.root.to_str().unwrap().to_owned(),
            shown: 3,
            total: 3
        }
    );

    let second = harness
        .done("list_directory", json!({ "offset": 1, "limit": 1 }))
        .await;
    assert!(
        second.output.contains("Entries 2 to 2 of 3."),
        "{}",
        second.output
    );
    assert!(second.output.contains("[file] b.txt"), "{}", second.output);
    assert!(
        second
            .output
            .contains("More follows: call again with offset 2."),
        "{}",
        second.output
    );
}

#[tokio::test]
async fn list_directory_stays_within_the_result_limit() {
    let harness = with_small_limit(Harness::new(), 4096);
    for index in 0..400 {
        harness.file(&format!("file-{index:03}.txt"), "x");
    }

    let done = harness
        .done("list_directory", json!({ "limit": 500 }))
        .await;

    assert!(done.output.len() <= 4096, "{} bytes", done.output.len());
    let ToolOutcome::Listed { shown, total, .. } = done.outcome else {
        panic!("not a listing: {:?}", done.outcome);
    };
    assert!(shown > 0 && shown < 400, "{shown}");
    assert_eq!(total, 400);
    assert!(
        done.output
            .contains(&format!("More follows: call again with offset {shown}.")),
        "{}",
        done.output
    );
}

#[tokio::test]
async fn list_directory_fails_on_files_and_missing_folders() {
    let harness = Harness::new();
    harness.file("a.txt", "a");

    assert_eq!(
        harness
            .failure("list_directory", json!({ "path": "a.txt" }))
            .await,
        ToolFailure::NotAFolder {
            path: harness.text("a.txt")
        }
    );
    assert_eq!(
        harness
            .failure("list_directory", json!({ "path": "missing" }))
            .await,
        ToolFailure::NotFound {
            path: harness.text("missing")
        }
    );
}

#[tokio::test]
async fn relative_paths_without_a_workspace_fail() {
    let mut harness = Harness::new();
    harness.context.workspace = None;

    assert_eq!(
        harness.failure("list_directory", json!({})).await,
        ToolFailure::NoWorkspace
    );
    assert_eq!(
        harness
            .failure("read_file", json!({ "path": "a.txt" }))
            .await,
        ToolFailure::NoWorkspace
    );
}

#[tokio::test]
async fn reads_declare_their_canonical_accesses() {
    let harness = Harness::new();
    harness.file("src/main.rs", "fn main() {}\n");

    let file = harness
        .prepare("read_file", &json!({ "path": "src/../src/main.rs" }))
        .await
        .unwrap();
    let folder = harness
        .prepare("list_directory", &json!({ "path": "src" }))
        .await
        .unwrap();

    let [
        Access::Path {
            path,
            mode,
            folder: is_folder,
            exists,
            identity,
            entries,
        },
    ] = file.accesses.as_slice()
    else {
        panic!("one path access: {:?}", file.accesses);
    };
    assert_eq!(path, &harness.text("src/main.rs"));
    assert_eq!(*mode, AccessMode::Read);
    assert!(!is_folder);
    assert!(exists);
    assert_ne!(identity.as_str(), "");
    assert_eq!(*entries, None);
    assert!(matches!(
        folder.accesses.as_slice(),
        [Access::Path { folder: true, .. }]
    ));
}

#[cfg(windows)]
#[tokio::test]
async fn network_paths_are_refused_while_preparing() {
    use mannie_types::tools::PathRefusal;

    let harness = Harness::new();

    assert_eq!(
        harness
            .failure(
                "read_file",
                json!({ "path": r"\\attacker.example\share\notes.txt" })
            )
            .await,
        ToolFailure::PathRefused {
            reason: PathRefusal::NetworkPath
        }
    );
    assert_eq!(
        harness
            .failure("list_directory", json!({ "path": r"\\?\C:\Windows" }))
            .await,
        ToolFailure::PathRefused {
            reason: PathRefusal::DevicePath
        }
    );
}

#[tokio::test]
async fn file_info_describes_files_folders_and_links() {
    let harness = Harness::new();
    harness.file("notes.txt", "hello");
    harness.folder("src");

    let file = harness
        .done("file_info", json!({ "path": "notes.txt" }))
        .await;
    let folder = harness.done("file_info", json!({ "path": "src" })).await;

    assert!(
        file.output.contains("Kind: file\nSize: 5 bytes\n"),
        "{}",
        file.output
    );
    assert!(file.output.contains("Read-only: no"), "{}", file.output);
    assert_eq!(
        file.outcome,
        ToolOutcome::Described {
            path: harness.text("notes.txt"),
            entry: EntryKind::File
        }
    );
    assert!(folder.output.contains("Kind: folder"), "{}", folder.output);
    assert_eq!(
        harness
            .failure("file_info", json!({ "path": "missing.txt" }))
            .await,
        ToolFailure::NotFound {
            path: harness.text("missing.txt")
        }
    );
}

#[cfg(unix)]
#[tokio::test]
async fn file_info_does_not_follow_a_link() {
    let harness = Harness::new();
    harness.file("target.txt", "content");
    std::os::unix::fs::symlink("target.txt", harness.path("link.txt")).unwrap();

    let done = harness
        .done("file_info", json!({ "path": "link.txt" }))
        .await;

    assert!(
        done.output
            .contains("Kind: link\nPoints to: target.txt (not followed)"),
        "{}",
        done.output
    );
    assert_eq!(
        done.outcome,
        ToolOutcome::Described {
            path: harness.text("link.txt"),
            entry: EntryKind::Link
        }
    );
}

#[tokio::test]
async fn find_files_matches_names_and_paths_and_skips_ignored_files() {
    let harness = Harness::new();
    harness.file(".gitignore", "target/\n");
    harness.file("src/main.rs", "");
    harness.file("src/bin/tool.rs", "");
    harness.file("target/build.rs", "");
    harness.file("README.md", "");

    let names = harness
        .done("find_files", json!({ "pattern": "*.rs" }))
        .await;
    let paths = harness
        .done("find_files", json!({ "pattern": "src/*.rs" }))
        .await;
    let everything = harness
        .done(
            "find_files",
            json!({ "pattern": "*.rs", "include_ignored": true }),
        )
        .await;

    let separator = std::path::MAIN_SEPARATOR;
    assert!(
        names.output.ends_with(&format!(
            "Matches 1 to 2.\nsrc{separator}bin{separator}tool.rs\nsrc{separator}main.rs\n"
        )),
        "{}",
        names.output
    );
    assert!(
        paths
            .output
            .ends_with(&format!("Matches 1 to 1.\nsrc{separator}main.rs\n")),
        "{}",
        paths.output
    );
    assert!(
        everything
            .output
            .contains(&format!("target{separator}build.rs")),
        "{}",
        everything.output
    );
    assert_eq!(
        names.outcome,
        ToolOutcome::Found {
            path: harness.root.to_str().unwrap().to_owned(),
            shown: 2,
            more: false
        }
    );
}

#[tokio::test]
async fn find_files_pages_its_matches() {
    let harness = Harness::new();
    for index in 0..5 {
        harness.file(&format!("file-{index}.txt"), "");
    }

    let first = harness
        .done("find_files", json!({ "pattern": "*.txt", "limit": 2 }))
        .await;
    let last = harness
        .done(
            "find_files",
            json!({ "pattern": "*.txt", "offset": 4, "limit": 2 }),
        )
        .await;

    assert!(
        first.output.contains(
            "Matches 1 to 2.\nfile-0.txt\nfile-1.txt\nMore follows: call again with offset 2.\n"
        ),
        "{}",
        first.output
    );
    assert!(matches!(
        first.outcome,
        ToolOutcome::Found {
            shown: 2,
            more: true,
            ..
        }
    ));
    assert!(
        last.output.ends_with("Matches 5 to 5.\nfile-4.txt\n"),
        "{}",
        last.output
    );
    assert!(matches!(
        last.outcome,
        ToolOutcome::Found {
            shown: 1,
            more: false,
            ..
        }
    ));
}

#[tokio::test]
async fn find_files_reports_invalid_patterns_and_no_matches() {
    let harness = Harness::new();

    assert!(matches!(
        harness
            .failure("find_files", json!({ "pattern": "src/[" }))
            .await,
        ToolFailure::InvalidPattern { .. }
    ));
    let none = harness
        .done("find_files", json!({ "pattern": "*.cpp" }))
        .await;
    assert!(
        none.output.ends_with("No files match.\n"),
        "{}",
        none.output
    );
}

#[tokio::test]
async fn search_text_finds_lines_by_text_and_by_expression() {
    let harness = Harness::new();
    harness.file("src/main.rs", "fn main() {\n    println!(\"Hello\");\n}\n");
    harness.file("src/lib.rs", "pub fn hello() {}\n");
    harness.file("notes.md", "hello from the notes\n");
    harness.file(
        "image.png",
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".as_slice(),
    );

    let literal = harness
        .done("search_text", json!({ "query": "hello" }))
        .await;
    let sensitive = harness
        .done(
            "search_text",
            json!({ "query": "Hello", "case_sensitive": true }),
        )
        .await;
    let expression = harness
        .done(
            "search_text",
            json!({ "query": r"fn \w+\(\)", "regex": true, "glob": "*.rs" }),
        )
        .await;
    let one_file = harness
        .done(
            "search_text",
            json!({ "query": "notes", "path": "notes.md" }),
        )
        .await;

    let separator = std::path::MAIN_SEPARATOR;
    assert!(
        literal.output.contains(&format!(
            "notes.md:1: hello from the notes\nsrc{separator}lib.rs:1: pub fn hello() {{}}\nsrc{separator}main.rs:2:     println!(\"Hello\");\n"
        )),
        "{}",
        literal.output
    );
    assert!(
        literal.output.contains("Files skipped: 1 binary."),
        "{}",
        literal.output
    );
    assert!(
        matches!(sensitive.outcome, ToolOutcome::Searched { shown: 1, .. }),
        "{}",
        sensitive.output
    );
    assert!(
        matches!(expression.outcome, ToolOutcome::Searched { shown: 2, .. }),
        "{}",
        expression.output
    );
    assert!(
        one_file.output.contains("notes.md:1: hello from the notes"),
        "{}",
        one_file.output
    );
}

#[tokio::test]
async fn search_text_pages_and_refuses_invalid_queries() {
    let harness = Harness::new();
    harness.file("log.txt", "match\n".repeat(5));

    let page = harness
        .done(
            "search_text",
            json!({ "query": "match", "limit": 2, "offset": 2 }),
        )
        .await;

    assert!(page.output.contains("Matches 3 to 4.\nlog.txt:3: match\nlog.txt:4: match\nMore follows: call again with offset 4.\n"), "{}", page.output);
    assert!(matches!(
        harness
            .failure("search_text", json!({ "query": "(", "regex": true }))
            .await,
        ToolFailure::InvalidPattern { .. }
    ));
    assert!(matches!(
        harness.failure("search_text", json!({ "query": "" })).await,
        ToolFailure::InvalidPattern { .. }
    ));
}

#[tokio::test]
async fn read_file_numbers_lines_and_names_the_revision() {
    let harness = Harness::new();
    harness.file("notes.txt", "first\nsecond\nthird\n");

    let done = harness
        .done("read_file", json!({ "path": "notes.txt" }))
        .await;
    let page = harness
        .done(
            "read_file",
            json!({ "path": "notes.txt", "offset": 2, "limit": 1 }),
        )
        .await;

    assert_eq!(
        done.output,
        format!(
            "File: {}\nRevision: sha256:{}\nLines 1 to 3 of 3.\n     1\tfirst\n     2\tsecond\n     3\tthird\n",
            harness.text("notes.txt"),
            sha256_hex(b"first\nsecond\nthird\n")
        )
    );
    assert!(
        page.output.ends_with(
            "Lines 2 to 2 of 3.\n     2\tsecond\nMore follows: call again with offset 3.\n"
        ),
        "{}",
        page.output
    );
    assert_eq!(
        page.outcome,
        ToolOutcome::Read {
            path: harness.text("notes.txt"),
            first_line: 2,
            last_line: 2,
            total_lines: 3
        }
    );
}

/// SHA-256 in hexadecimal, computed independently of the tools.
fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(bytes))
}

#[tokio::test]
async fn read_file_handles_encodings_and_line_endings() {
    let harness = Harness::new();
    harness.file("windows.txt", "one\r\ntwo\r\n");
    harness.file("utf16.txt", b"\xff\xfeh\x00i\x00\n\x00".as_slice());
    harness.file("latin1.txt", b"caf\xe9\n".as_slice());
    harness.file("binary.bin", b"\x00\x01\x02".as_slice());

    let crlf = harness
        .done("read_file", json!({ "path": "windows.txt" }))
        .await;
    let utf16 = harness
        .done("read_file", json!({ "path": "utf16.txt" }))
        .await;

    assert!(
        crlf.output.contains("CRLF line endings, shown as LF"),
        "{}",
        crlf.output
    );
    assert!(
        crlf.output.ends_with("     1\tone\n     2\ttwo\n"),
        "{}",
        crlf.output
    );
    assert!(
        utf16.output.contains("UTF-16 with a byte order mark"),
        "{}",
        utf16.output
    );
    assert!(utf16.output.ends_with("     1\thi\n"), "{}", utf16.output);
    assert_eq!(
        harness
            .failure("read_file", json!({ "path": "latin1.txt" }))
            .await,
        ToolFailure::UnsupportedEncoding {
            path: harness.text("latin1.txt")
        }
    );
    assert_eq!(
        harness
            .failure("read_file", json!({ "path": "binary.bin" }))
            .await,
        ToolFailure::BinaryFile {
            path: harness.text("binary.bin")
        }
    );
}

#[tokio::test]
async fn read_file_cuts_long_lines_and_refuses_folders_and_large_files() {
    let harness = Harness::new();
    harness.file("long.txt", format!("{}\n", "x".repeat(2_500)));
    harness.folder("src");
    harness.file("large.txt", vec![b'a'; 16 * 1024 * 1024 + 1]);

    let long = harness
        .done("read_file", json!({ "path": "long.txt" }))
        .await;

    assert!(
        long.output.ends_with(&format!(
            "     1\t{} [cut: 500 more characters]\n",
            "x".repeat(2_000)
        )),
        "{}",
        long.output.len()
    );
    assert_eq!(
        harness.failure("read_file", json!({ "path": "src" })).await,
        ToolFailure::NotAFile {
            path: harness.text("src")
        }
    );
    assert_eq!(
        harness
            .failure("read_file", json!({ "path": "large.txt" }))
            .await,
        ToolFailure::TooLarge {
            limit_bytes: 16 * 1024 * 1024
        }
    );
}

#[tokio::test]
async fn read_file_of_an_empty_file_says_so() {
    let harness = Harness::new();
    harness.file("empty.txt", "");

    let done = harness
        .done("read_file", json!({ "path": "empty.txt" }))
        .await;

    assert!(
        done.output.ends_with("The file is empty.\n"),
        "{}",
        done.output
    );
}

#[tokio::test]
async fn read_tool_output_needs_an_archive_of_the_conversation() {
    let harness = Harness::new();

    assert_eq!(
        harness
            .failure(
                "read_tool_output",
                json!({ "output_id": "0199e0f2-0000-7000-a000-000000000009" })
            )
            .await,
        ToolFailure::OutputNotFound
    );
    assert_eq!(
        harness
            .failure("read_tool_output", json!({ "output_id": "not-an-id" }))
            .await,
        ToolFailure::OutputNotFound
    );
}

#[test]
fn paths_in_this_suite_are_absolute() {
    let harness = Harness::new();

    assert!(Path::new(&harness.text("a.txt")).is_absolute());
}

/// The revision that the tools name for some bytes.
fn revision_of(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

/// Links a folder: a symbolic link on Unix, a junction on Windows, which needs
/// no privilege.
fn link_folder(link: &Path, target: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");
    }
}

/// The path, mode and folder flag of each path access.
fn path_accesses(accesses: &[Access]) -> Vec<(String, AccessMode, bool)> {
    accesses
        .iter()
        .map(|access| match access {
            Access::Path {
                path, mode, folder, ..
            } => (path.clone(), *mode, *folder),
            Access::Command { .. } | Access::Project { .. } => {
                panic!("not a path access: {access:?}")
            }
        })
        .collect()
}

/// The failure that a finished call reports.
fn failure_of(done: Done) -> ToolFailure {
    match done.outcome {
        ToolOutcome::Failed { failure } => failure,
        other => panic!("expected a failure, got {other:?}: {}", done.output),
    }
}

#[tokio::test]
async fn write_file_creates_a_file_and_the_folders_above_it() {
    let harness = Harness::new();
    let arguments = json!({ "path": "src/new/mod.rs", "content": "pub fn a() {}\n" });

    let prepared = harness.prepare("write_file", &arguments).await.unwrap();
    let done = harness.done("write_file", arguments).await;

    assert_eq!(
        path_accesses(&prepared.accesses),
        [
            (harness.text("src"), AccessMode::Create, true),
            (harness.text("src/new"), AccessMode::Create, true),
            (harness.text("src/new/mod.rs"), AccessMode::Create, false),
        ]
    );
    assert_eq!(
        fs::read_to_string(harness.path("src/new/mod.rs")).unwrap(),
        "pub fn a() {}\n"
    );
    assert_eq!(
        done.outcome,
        ToolOutcome::Written {
            path: harness.text("src/new/mod.rs"),
            created: true,
            bytes: 14
        }
    );
    assert!(
        done.output
            .contains(&format!("Revision: {}", revision_of(b"pub fn a() {}\n"))),
        "{}",
        done.output
    );
}

#[tokio::test]
async fn write_file_never_overwrites_without_a_revision() {
    let harness = Harness::new();
    harness.file("notes.txt", "theirs");

    assert_eq!(
        harness
            .failure(
                "write_file",
                json!({ "path": "notes.txt", "content": "mine" })
            )
            .await,
        ToolFailure::AlreadyExists {
            path: harness.text("notes.txt")
        }
    );
    assert_eq!(
        fs::read_to_string(harness.path("notes.txt")).unwrap(),
        "theirs"
    );
}

#[tokio::test]
async fn write_file_replaces_only_the_revision_it_names() {
    let harness = Harness::new();
    harness.file("notes.txt", "old\n");

    let stale = harness
        .failure(
            "write_file",
            json!({ "path": "notes.txt", "content": "new\n", "expected_revision": "sha256:00" }),
        )
        .await;
    assert_eq!(
        stale,
        ToolFailure::RevisionMismatch {
            current: revision_of(b"old\n")
        }
    );
    assert_eq!(
        fs::read_to_string(harness.path("notes.txt")).unwrap(),
        "old\n"
    );

    let done = harness
        .done(
            "write_file",
            json!({ "path": "notes.txt", "content": "new\n", "expected_revision": revision_of(b"old\n") }),
        )
        .await;
    assert_eq!(
        fs::read_to_string(harness.path("notes.txt")).unwrap(),
        "new\n"
    );
    assert_eq!(
        done.outcome,
        ToolOutcome::Written {
            path: harness.text("notes.txt"),
            created: false,
            bytes: 4
        }
    );
}

#[tokio::test]
async fn write_file_keeps_the_encoding_and_the_line_endings() {
    let harness = Harness::new();
    harness.file("windows.txt", "one\r\ntwo\r\n");
    harness.file("utf16.txt", b"\xff\xfeh\x00i\x00\n\x00".as_slice());

    harness
        .done(
            "write_file",
            json!({ "path": "windows.txt", "content": "uno\ndos\n", "expected_revision": revision_of(b"one\r\ntwo\r\n") }),
        )
        .await;
    harness
        .done(
            "write_file",
            json!({ "path": "utf16.txt", "content": "ok\n", "expected_revision": revision_of(b"\xff\xfeh\x00i\x00\n\x00") }),
        )
        .await;

    assert_eq!(
        fs::read(harness.path("windows.txt")).unwrap(),
        b"uno\r\ndos\r\n"
    );
    assert_eq!(
        fs::read(harness.path("utf16.txt")).unwrap(),
        b"\xff\xfeo\x00k\x00\n\x00"
    );
}

#[tokio::test]
async fn write_file_refuses_large_content_and_read_only_files() {
    let harness = Harness::new();
    harness.file("locked.txt", "locked");
    let path = harness.path("locked.txt");
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();

    assert_eq!(
        harness
            .failure(
                "write_file",
                json!({ "path": "big.txt", "content": "x".repeat(1024 * 1024 + 1) }),
            )
            .await,
        ToolFailure::TooLarge {
            limit_bytes: 1024 * 1024
        }
    );
    assert_eq!(
        harness
            .failure(
                "write_file",
                json!({ "path": "locked.txt", "content": "x", "expected_revision": revision_of(b"locked") }),
            )
            .await,
        ToolFailure::PermissionDenied {
            path: harness.text("locked.txt")
        }
    );
}

#[tokio::test]
async fn edit_file_replaces_exact_text_and_names_the_new_revision() {
    let harness = Harness::new();
    let old = "fn main() {\n    old();\n}\n";
    harness.file("main.rs", old);

    let done = harness
        .done(
            "edit_file",
            json!({
                "path": "main.rs",
                "expected_revision": revision_of(old.as_bytes()),
                "edits": [{ "old_text": "old()", "new_text": "new()" }],
            }),
        )
        .await;

    let new = "fn main() {\n    new();\n}\n";
    assert_eq!(fs::read_to_string(harness.path("main.rs")).unwrap(), new);
    assert_eq!(
        done.outcome,
        ToolOutcome::Edited {
            path: harness.text("main.rs"),
            replacements: 1
        }
    );
    assert!(
        done.output
            .contains(&format!("Revision: {}", revision_of(new.as_bytes()))),
        "{}",
        done.output
    );
}

#[tokio::test]
async fn a_failing_edit_changes_nothing() {
    let harness = Harness::new();
    let text = "a\nb\na\n";
    harness.file("list.txt", text);
    let revision = revision_of(text.as_bytes());
    let edit =
        |edits: Value| json!({ "path": "list.txt", "expected_revision": revision, "edits": edits });

    assert_eq!(
        harness
            .failure(
                "edit_file",
                edit(json!([{ "old_text": "c", "new_text": "d" }]))
            )
            .await,
        ToolFailure::EditNotFound { edit: 1 }
    );
    assert_eq!(
        harness
            .failure(
                "edit_file",
                edit(json!([{ "old_text": "b", "new_text": "B" }, { "old_text": "a", "new_text": "A" }])),
            )
            .await,
        ToolFailure::EditAmbiguous {
            edit: 2,
            matches: 2
        }
    );
    assert!(matches!(
        harness
            .failure(
                "edit_file",
                json!({ "path": "list.txt", "expected_revision": "sha256:00", "edits": [{ "old_text": "b", "new_text": "B" }] }),
            )
            .await,
        ToolFailure::RevisionMismatch { .. }
    ));
    assert_eq!(fs::read_to_string(harness.path("list.txt")).unwrap(), text);
}

#[tokio::test]
async fn edit_file_checks_its_edits_before_anything_else() {
    let harness = Harness::new();
    harness.file("list.txt", "a\n");

    for edits in [json!([]), json!([{ "old_text": "", "new_text": "x" }])] {
        let refused = harness
            .prepare(
                "edit_file",
                &json!({ "path": "list.txt", "expected_revision": "sha256:00", "edits": edits }),
            )
            .await;
        assert!(
            matches!(refused, Err(PrepareError::InvalidArguments { .. })),
            "{refused:?}"
        );
    }
}

#[tokio::test]
async fn create_directory_creates_the_missing_folders_once() {
    let harness = Harness::new();
    harness.file("file.txt", "x");

    let created = harness
        .done("create_directory", json!({ "path": "a/b/c" }))
        .await;
    let again = harness
        .done("create_directory", json!({ "path": "a/b/c" }))
        .await;

    assert!(harness.path("a/b/c").is_dir());
    assert_eq!(
        created.outcome,
        ToolOutcome::FolderCreated {
            path: harness.text("a/b/c"),
            created: 3
        }
    );
    assert_eq!(
        again.outcome,
        ToolOutcome::FolderCreated {
            path: harness.text("a/b/c"),
            created: 0
        }
    );
    assert_eq!(
        harness
            .failure("create_directory", json!({ "path": "file.txt" }))
            .await,
        ToolFailure::AlreadyExists {
            path: harness.text("file.txt")
        }
    );
}

#[tokio::test]
async fn copy_path_copies_files_and_folders_without_overwriting() {
    let harness = Harness::new();
    harness.file("src/a.txt", "a");
    harness.file("src/deep/b.txt", "b");

    let folder = harness
        .done(
            "copy_path",
            json!({ "source": "src", "destination": "copy" }),
        )
        .await;
    let file = harness
        .done(
            "copy_path",
            json!({ "source": "src/a.txt", "destination": "a-copy.txt" }),
        )
        .await;

    assert_eq!(fs::read_to_string(harness.path("copy/a.txt")).unwrap(), "a");
    assert_eq!(
        fs::read_to_string(harness.path("copy/deep/b.txt")).unwrap(),
        "b"
    );
    assert_eq!(fs::read_to_string(harness.path("a-copy.txt")).unwrap(), "a");
    assert_eq!(
        folder.outcome,
        ToolOutcome::Copied {
            source: harness.text("src"),
            destination: harness.text("copy"),
            files: 2
        }
    );
    assert!(matches!(file.outcome, ToolOutcome::Copied { files: 1, .. }));
    assert_eq!(
        harness
            .failure(
                "copy_path",
                json!({ "source": "src/a.txt", "destination": "copy/a.txt" })
            )
            .await,
        ToolFailure::AlreadyExists {
            path: harness.text("copy/a.txt")
        }
    );
    let into_itself = harness
        .prepare(
            "copy_path",
            &json!({ "source": "src", "destination": "src/inner" }),
        )
        .await;
    assert!(
        matches!(into_itself, Err(PrepareError::InvalidArguments { .. })),
        "{into_itself:?}"
    );
}

#[tokio::test]
async fn copy_path_refuses_links_in_the_source() {
    let harness = Harness::new();
    harness.folder("outside");
    harness.file("src/a.txt", "a");
    link_folder(&harness.path("src/escape"), &harness.path("outside"));

    assert_eq!(
        harness
            .failure(
                "copy_path",
                json!({ "source": "src", "destination": "copy" })
            )
            .await,
        ToolFailure::LinkInSource {
            path: harness.text("src/escape")
        }
    );
    assert!(!harness.path("copy").exists());
}

#[tokio::test]
async fn move_path_moves_files_and_folders_and_never_replaces() {
    let harness = Harness::new();
    harness.file("a.txt", "a");
    harness.file("taken.txt", "taken");
    harness.file("src/inner.txt", "inner");

    harness
        .done(
            "move_path",
            json!({ "source": "a.txt", "destination": "b.txt" }),
        )
        .await;
    let folder = harness
        .done(
            "move_path",
            json!({ "source": "src", "destination": "moved" }),
        )
        .await;

    assert_eq!(fs::read_to_string(harness.path("b.txt")).unwrap(), "a");
    assert!(!harness.path("a.txt").exists());
    assert_eq!(
        fs::read_to_string(harness.path("moved/inner.txt")).unwrap(),
        "inner"
    );
    assert_eq!(
        folder.outcome,
        ToolOutcome::Moved {
            source: harness.text("src"),
            destination: harness.text("moved")
        }
    );
    assert_eq!(
        harness
            .failure(
                "move_path",
                json!({ "source": "b.txt", "destination": "taken.txt" })
            )
            .await,
        ToolFailure::AlreadyExists {
            path: harness.text("taken.txt")
        }
    );
    assert_eq!(
        fs::read_to_string(harness.path("taken.txt")).unwrap(),
        "taken"
    );
}

#[tokio::test]
async fn delete_path_deletes_files_folders_and_links() {
    let harness = Harness::new();
    harness.file("a.txt", "a");
    harness.file("build/one.txt", "1");
    harness.file("build/deep/two.txt", "2");
    harness.file("real/keep.txt", "keep");
    link_folder(&harness.path("link"), &harness.path("real"));

    let file = harness
        .done("delete_path", json!({ "path": "a.txt" }))
        .await;
    let refused = harness
        .failure("delete_path", json!({ "path": "build" }))
        .await;
    let prepared = harness
        .prepare(
            "delete_path",
            &json!({ "path": "build", "recursive": true }),
        )
        .await
        .unwrap();
    let folder = harness
        .done("delete_path", json!({ "path": "build", "recursive": true }))
        .await;
    harness.done("delete_path", json!({ "path": "link" })).await;

    assert!(!harness.path("a.txt").exists());
    assert_eq!(
        file.outcome,
        ToolOutcome::Deleted {
            path: harness.text("a.txt"),
            files: 1,
            folders: 0
        }
    );
    assert_eq!(
        refused,
        ToolFailure::FolderNotEmpty {
            path: harness.text("build")
        }
    );
    assert!(matches!(
        prepared.accesses.as_slice(),
        [Access::Path {
            mode: AccessMode::Delete,
            folder: true,
            entries: Some(EntryCount {
                files: 2,
                folders: 1
            }),
            ..
        }]
    ));
    assert_eq!(
        folder.outcome,
        ToolOutcome::Deleted {
            path: harness.text("build"),
            files: 2,
            folders: 2
        }
    );
    assert!(!harness.path("build").exists());
    assert!(!harness.path("link").exists());
    assert_eq!(
        fs::read_to_string(harness.path("real/keep.txt")).unwrap(),
        "keep"
    );
}

#[tokio::test]
async fn a_file_replaced_after_the_authorization_is_not_changed() {
    let harness = Harness::new();
    harness.file("notes.txt", "old");
    let arguments =
        json!({ "path": "notes.txt", "content": "new", "expected_revision": revision_of(b"old") });
    let prepared = harness.prepare("write_file", &arguments).await.unwrap();

    // Another program replaces the file with one of the same content.
    harness.file("other.txt", "old");
    fs::rename(harness.path("other.txt"), harness.path("notes.txt")).unwrap();
    let done = harness
        .execute(
            "write_file",
            &arguments,
            prepared.accesses,
            StopSignal::new(),
        )
        .await;

    assert_eq!(failure_of(done), ToolFailure::PathChanged);
    assert_eq!(
        fs::read_to_string(harness.path("notes.txt")).unwrap(),
        "old"
    );
}

#[tokio::test]
async fn a_folder_swapped_for_a_link_after_the_authorization_is_not_changed() {
    let harness = Harness::new();
    harness.folder("project");
    harness.folder("elsewhere");
    let arguments = json!({ "path": "project/new.txt", "content": "x" });
    let prepared = harness.prepare("write_file", &arguments).await.unwrap();

    fs::rename(harness.path("project"), harness.path("project-old")).unwrap();
    link_folder(&harness.path("project"), &harness.path("elsewhere"));
    let done = harness
        .execute(
            "write_file",
            &arguments,
            prepared.accesses,
            StopSignal::new(),
        )
        .await;

    assert_eq!(
        failure_of(done),
        ToolFailure::PathThroughLink {
            resolved: harness.text("elsewhere/new.txt")
        }
    );
    assert!(!harness.path("elsewhere/new.txt").exists());
}

#[tokio::test]
async fn a_folder_whose_content_changed_is_not_deleted() {
    let harness = Harness::new();
    harness.file("build/one.txt", "1");
    let arguments = json!({ "path": "build", "recursive": true });
    let prepared = harness.prepare("delete_path", &arguments).await.unwrap();

    harness.file("build/added.txt", "2");
    let done = harness
        .execute(
            "delete_path",
            &arguments,
            prepared.accesses,
            StopSignal::new(),
        )
        .await;

    assert_eq!(failure_of(done), ToolFailure::PathChanged);
    assert!(harness.path("build/added.txt").exists());
}

/// A command in the platform's shell: PowerShell on Windows, `sh` elsewhere.
#[cfg(any(windows, target_os = "linux"))]
fn shell<'a>(windows: &'a str, unix: &'a str) -> &'a str {
    if cfg!(windows) { windows } else { unix }
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn run_command_reports_the_exit_code_and_both_streams() {
    let harness = Harness::new();
    let command = shell(
        "Write-Output 'hello'; [Console]::Error.WriteLine('oops'); exit 3",
        "echo hello; echo oops >&2; exit 3",
    );

    let done = harness
        .done("run_command", json!({ "command": command }))
        .await;

    assert!(done.output.starts_with("Exit code: 3\n"), "{}", done.output);
    assert!(
        done.output.contains("Standard output (") && done.output.contains("hello"),
        "{}",
        done.output
    );
    assert!(done.output.contains("Standard error ("), "{}", done.output);
    let (_, error_part) = done.output.split_once("Standard error").unwrap();
    assert!(error_part.contains("oops"), "{}", done.output);
    assert!(matches!(
        done.outcome,
        ToolOutcome::CommandRan {
            ending: CommandEnding::Exited,
            exit_code: Some(3),
            ..
        }
    ));
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn run_command_runs_in_the_working_directory_with_unicode_output() {
    let harness = Harness::new();
    harness.folder("sub");
    let command = shell(
        "(Get-Location).Path; Write-Output 'Grüße, мир'",
        "pwd; printf 'Grüße, мир\\n'",
    );

    let done = harness
        .done(
            "run_command",
            json!({ "command": command, "working_directory": "sub" }),
        )
        .await;

    assert!(
        done.output.contains(&harness.text("sub")),
        "{}",
        done.output
    );
    assert!(done.output.contains("Grüße, мир"), "{}", done.output);
    assert!(!done.output.contains("U+FFFD"), "{}", done.output);
}

#[cfg(windows)]
#[tokio::test]
async fn run_command_runs_multi_line_powershell() {
    let harness = Harness::new();
    let command =
        "$text = @'\nfirst line\nsecond line\n'@\nif ($true) {\n  Write-Output $text\n}\n";

    let done = harness
        .done("run_command", json!({ "command": command }))
        .await;

    assert!(done.output.starts_with("Exit code: 0\n"), "{}", done.output);
    assert!(
        done.output.contains("first line\nsecond line"),
        "{}",
        done.output
    );
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn run_command_receives_only_allowed_variables() {
    let mut harness = Harness::new();
    let mut source: Vec<(std::ffi::OsString, std::ffi::OsString)> = std::env::vars_os()
        .filter(|(name, _)| name != "MANNIE_TEST_SECRET")
        .collect();
    source.push(("MANNIE_TEST_SECRET".into(), "planted-value".into()));
    harness.context.environment = Arc::new(CommandEnvironment::new(&[], source));
    let command = shell(
        "Get-ChildItem env: | ForEach-Object { $_.Name + '=' + $_.Value }",
        "env",
    );

    let done = harness
        .done("run_command", json!({ "command": command }))
        .await;

    assert!(!done.output.contains("planted-value"), "{}", done.output);
    assert!(
        !done.output.contains("MANNIE_TEST_SECRET"),
        "{}",
        done.output
    );
    assert!(
        done.output.to_uppercase().contains("PATH="),
        "{}",
        done.output
    );
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn long_output_is_cut_and_archived() {
    let harness = with_small_limit(Harness::new(), 8 * 1024);
    let command = shell(
        "1..5000 | ForEach-Object { \"line $_\" }",
        "i=1; while [ $i -le 5000 ]; do echo \"line $i\"; i=$((i+1)); done",
    );

    let done = harness
        .done("run_command", json!({ "command": command }))
        .await;

    assert!(done.output.len() <= 8 * 1024, "{} bytes", done.output.len());
    assert!(done.output.contains("line 1\n"), "{}", done.output);
    assert!(done.output.contains("line 5000"), "{}", done.output);
    assert!(
        done.output.contains(
            "left out: the whole output is archived; read it with read_tool_output, output_id 0199e0f2-0000-7000-a000-000000000001"
        ),
        "{}",
        done.output
    );
    let [archive] = done.archives.as_slice() else {
        panic!("one archive: {:?}", done.archives.len());
    };
    assert_eq!(archive.name, "stdout");
    assert_eq!(archive.content.lines().count(), 5000);
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn a_command_past_its_timeout_is_ended() {
    let harness = Harness::new();
    let command = shell("Start-Sleep -Seconds 30", "sleep 30");
    let started = std::time::Instant::now();

    let done = harness
        .done(
            "run_command",
            json!({ "command": command, "timeout_seconds": 1 }),
        )
        .await;

    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    assert!(
        done.output.starts_with("Timed out after 1 s"),
        "{}",
        done.output
    );
    assert!(matches!(
        done.outcome,
        ToolOutcome::CommandRan {
            ending: CommandEnding::TimedOut,
            exit_code: None,
            ..
        }
    ));
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn stop_ends_a_command_within_a_second() {
    let harness = Harness::new();
    let arguments = json!({ "command": shell("Start-Sleep -Seconds 30", "sleep 30") });
    let prepared = harness.prepare("run_command", &arguments).await.unwrap();
    let stop = StopSignal::new();
    let stopper = stop.clone();
    let execution = harness.execute("run_command", &arguments, prepared.accesses, stop);
    let stopping = async {
        // PowerShell takes a while to start; the stop comes once it runs.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let stopped_at = std::time::Instant::now();
        stopper.stop(StopReason::Stopped);
        stopped_at
    };

    let (done, stopped_at) = tokio::join!(execution, stopping);

    assert!(
        stopped_at.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        stopped_at.elapsed()
    );
    assert!(matches!(
        done.outcome,
        ToolOutcome::CommandRan {
            ending: CommandEnding::Stopped,
            ..
        }
    ));
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn commands_need_an_existing_working_directory_and_some_text() {
    let harness = Harness::new();

    assert_eq!(
        harness
            .failure(
                "run_command",
                json!({ "command": "echo", "working_directory": "missing" })
            )
            .await,
        ToolFailure::NotFound {
            path: harness.text("missing")
        }
    );
    let empty = harness
        .prepare("run_command", &json!({ "command": "  " }))
        .await;
    assert!(
        matches!(empty, Err(PrepareError::InvalidArguments { .. })),
        "{empty:?}"
    );
}

/// Moves that a test records; each one answers with the project.
#[derive(Default)]
struct RecordedMoves {
    projects: Vec<ProjectView>,
    moves: std::sync::Mutex<Vec<(ConversationId, ProjectId)>>,
}

impl ProjectMoves for RecordedMoves {
    fn move_into(
        &self,
        conversation: ConversationId,
        project: ProjectId,
    ) -> futures_util::future::BoxFuture<'static, Result<ProjectView, MoveFailure>> {
        let found = self
            .projects
            .iter()
            .find(|candidate| candidate.id == project)
            .cloned();
        self.moves.lock().unwrap().push((conversation, project));
        Box::pin(async move { found.ok_or(MoveFailure::ProjectNotFound) })
    }
}

fn mannie_project(folder: String) -> ProjectView {
    ProjectView {
        id: ProjectId::parse("0199e0f2-0000-7000-b000-000000000001").unwrap(),
        name: "mannie".to_owned(),
        folder,
        created_at: UtcMillis::EPOCH,
    }
}

#[tokio::test]
async fn move_to_project_moves_a_general_conversation_into_the_project_it_names() {
    let mut harness = Harness::new();
    let project = mannie_project(harness.text("project"));
    let moves = Arc::new(RecordedMoves {
        projects: vec![project.clone()],
        ..RecordedMoves::default()
    });
    harness.context.projects = ProjectFacts {
        current: None,
        all: Arc::from(vec![project.clone()]),
        moves: Arc::<RecordedMoves>::clone(&moves),
    };
    let arguments = json!({ "project": "Mannie" });

    let prepared = harness
        .prepare("move_to_project", &arguments)
        .await
        .unwrap();
    let done = harness
        .execute(
            "move_to_project",
            &arguments,
            prepared.accesses.clone(),
            StopSignal::new(),
        )
        .await;
    let unknown = harness
        .failure("move_to_project", json!({ "project": "elsewhere" }))
        .await;
    harness.context.projects.current = Some(project.clone());
    let moved_again = harness.failure("move_to_project", arguments).await;

    assert_eq!(
        prepared.accesses,
        vec![Access::Project {
            id: project.id.clone(),
            name: "mannie".to_owned(),
            folder: project.folder.clone(),
        }]
    );
    assert_eq!(
        done.outcome,
        ToolOutcome::MovedToProject {
            name: "mannie".to_owned(),
            folder: project.folder.clone(),
        }
    );
    assert!(
        done.output.contains("now belongs to the project mannie"),
        "{}",
        done.output
    );
    assert_eq!(
        *moves.moves.lock().unwrap(),
        vec![(harness.context.conversation_id.clone(), project.id.clone())]
    );
    assert_eq!(
        unknown,
        ToolFailure::ProjectNotFound {
            project: "elsewhere".to_owned()
        }
    );
    assert_eq!(
        moved_again,
        ToolFailure::AlreadyInProject {
            name: "mannie".to_owned()
        }
    );
}

#[test]
fn move_to_project_is_offered_only_to_a_general_conversation_while_projects_exist() {
    let registry = ToolRegistry::with_system_tools();
    let tool = registry.get("move_to_project").unwrap();
    let project = mannie_project("/projects/mannie".to_owned());
    let projects = [project.clone()];

    assert!(tool.offered(&Availability {
        project: None,
        projects: &projects,
    }));
    assert!(!tool.offered(&Availability {
        project: None,
        projects: &[],
    }));
    assert!(!tool.offered(&Availability {
        project: Some(&project),
        projects: &projects,
    }));
    assert!(registry.get("read_file").unwrap().offered(&Availability {
        project: Some(&project),
        projects: &[],
    }));
}
