//! The mention provider `git` (ADR-0032; plugins/git/docs/mentions.md):
//! "@" in the message box lists the uncommitted changes, the newest local
//! branches and the newest commits of the current branch, and a mention
//! gives the model what it names: the changes' diff, a commit with its
//! patch, or a branch's commits and the files that they change.
//!
//! A mention never asks (ADR-0032, decision 8), so every run of git is
//! checked first as a model's reads of the repository's folders would be in
//! the conversation (ADR-0038, decision 10). A read that would ask gives the
//! model a note instead of the content. The keys come from the webview, so
//! each is checked again before git runs (ADR-0004).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{MentionEntry, MentionSummary};
use mannie_plugin_sdk::points::mentions::{
    MentionContent, MentionContext, MentionProvider, ProviderSuggestions, RenderedMention,
};
use mannie_plugin_sdk::services::processes::ReadPlace;

use crate::arguments::{check_branch, is_object_id};
use crate::failure::GitFailure;
use crate::git::Git;
use crate::layout::Layout;
use crate::parse::diff::parse_numstat;
use crate::parse::log::{LOG_FORMAT, parse_log};
use crate::texts;
use crate::tools::Reading;
use crate::tools::diff::{PATCH_OPTIONS, stat_line};
use crate::tools::show::shown_commit;
use crate::tools::status::{read_status, status_text};

/// The provider's ID, which is the plugin's.
pub(crate) const PROVIDER_ID: &str = "git";

/// The key and the variant of the uncommitted changes.
const CHANGES: &str = "changes";

/// The variant of a commit's entries, and the prefix of their keys.
const COMMIT: &str = "commit";

/// The variant of a branch's entries, and the prefix of their keys.
const BRANCH: &str = "branch";

/// The summary of a mention that the conversation may not read yet.
const NOT_READ: &str = "not_read";

/// The newest commits that the list offers.
const LISTED_COMMITS: u32 = 50;

/// The commits of a branch that a mention gives at most.
const BRANCH_COMMITS: u32 = 50;

/// The entries that, with a workspace this many folders deep, mark a
/// repository: the core's offering of the tools looks as far up.
const MAX_MARKER_DEPTH: usize = 32;

/// The commits, branches and changes of the repository, as "@" lists them.
#[derive(Debug)]
pub(crate) struct GitMentions {
    git: Git,
}

impl GitMentions {
    pub(crate) fn new(git: Git) -> Self {
        Self { git }
    }
}

/// Whether `folder`, or a folder above it, holds `.git`, without following
/// links, as the core decides where the tools are offered.
pub(crate) fn in_repository(folder: &Path) -> bool {
    folder
        .ancestors()
        .take(MAX_MARKER_DEPTH)
        .any(|ancestor| std::fs::symlink_metadata(ancestor.join(".git")).is_ok())
}

/// The place whose policy checks a mention's reads of `folders`: the
/// conversation, or the project of a message that starts one.
fn read_place(context: &MentionContext, folders: Vec<std::path::PathBuf>) -> Option<ReadPlace> {
    if let Some(conversation) = context.conversation() {
        return Some(ReadPlace::Conversation {
            conversation: conversation.clone(),
            folders,
        });
    }
    context.project().map(|project| ReadPlace::Project {
        project: project.clone(),
        folders,
    })
}

/// What a key names, once checked.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Named {
    Changes,
    Commit(String),
    Branch(String),
}

impl Named {
    /// The key that the webview sent, checked again.
    fn parse(key: &str) -> Option<Self> {
        if key == CHANGES {
            return Some(Self::Changes);
        }
        if let Some(oid) = key.strip_prefix("commit:") {
            return is_object_id(oid).then(|| Self::Commit(oid.to_owned()));
        }
        let name = key.strip_prefix("branch:")?;
        is_branch_name(name).then(|| Self::Branch(name.to_owned()))
    }

    fn key(&self) -> String {
        match self {
            Self::Changes => CHANGES.to_owned(),
            Self::Commit(oid) => format!("commit:{oid}"),
            Self::Branch(name) => format!("branch:{name}"),
        }
    }
}

/// Whether a text can be a branch's short name: no part that a revision
/// reads as an operator, and nothing that git reads as an option.
fn is_branch_name(name: &str) -> bool {
    check_branch("branch", name).is_ok()
}

impl MentionProvider for GitMentions {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn offered(&self, context: &MentionContext) -> bool {
        context.workspace().is_some_and(in_repository)
    }

    fn suggest<'a>(
        &'a self,
        context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        Box::pin(async move {
            let reading = self
                .reading(context, 0)
                .await
                .map_err(|failure| failure.code().to_owned())?;
            suggestions(&reading, query, limit)
                .await
                .map_err(|failure| failure.code().to_owned())
        })
    }

    fn resolve<'a>(
        &'a self,
        context: &'a MentionContext,
        key: &'a str,
        max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        Box::pin(async move {
            let named = Named::parse(key).ok_or_else(|| "invalid_key".to_owned())?;
            let outcome = match self.reading(context, max_bytes).await {
                Ok(reading) => read_named(&reading, &named).await,
                Err(failure) => Err(failure),
            };
            let mentioned = match outcome {
                Ok(mentioned) => mentioned,
                // A mention never asks, so the model reads why it got nothing.
                Err(GitFailure::ReadNotAllowed { .. }) => Mentioned::not_read(&named),
                Err(failure) => return Err(failure.code().to_owned()),
            };
            let given: Box<dyn MentionContent> = Box::new(mentioned);
            Ok(given)
        })
    }
}

impl GitMentions {
    /// The repository of the workspace, every run checked as the place's
    /// reads of its folders.
    async fn reading(&self, context: &MentionContext, room: usize) -> Result<Reading, GitFailure> {
        let workspace = context
            .workspace()
            .ok_or_else(|| GitFailure::NotARepository {
                folder: String::new(),
            })?;
        let checks = read_place(context, vec![workspace.to_path_buf()]);
        let layout = Layout::discover(&self.git, workspace, checks).await?;
        let checks = read_place(context, layout.read_folders());
        Reading::open(&self.git, layout, room, checks).await
    }
}

/// The entries that match `query`: the uncommitted changes, then the
/// branches, then the commits of HEAD, each matched by its name, subject or
/// hash without regard to case.
async fn suggestions(
    reading: &Reading,
    query: &str,
    limit: usize,
) -> Result<ProviderSuggestions, GitFailure> {
    let query = query.trim().to_lowercase();
    let mut entries = Vec::new();
    let mentions_changes = "uncommitted changes".contains(&query) || query.is_empty();
    if mentions_changes {
        let status = read_status(reading).await?;
        if !status.entries.is_empty() {
            entries.push(entry(
                &Named::Changes,
                "Uncommitted changes",
                "@changes",
                CHANGES,
            ));
        }
    }
    let branches = reading
        .git
        .output(reading.run([
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            "refs/heads",
        ]))
        .await?;
    for name in branches.text().lines().filter(|name| is_branch_name(name)) {
        if name.to_lowercase().contains(&query) {
            let named = Named::Branch(name.to_owned());
            entries.push(entry(&named, name, &format!("@{name}"), BRANCH));
        }
    }
    let log = reading
        .git
        .run(reading.run([
            "log".to_owned(),
            "-z".to_owned(),
            LOG_FORMAT.to_owned(),
            format!("--max-count={LISTED_COMMITS}"),
            "HEAD".to_owned(),
        ]))
        .await?;
    // A branch without commits has no log.
    let commits = if log.succeeded() {
        parse_log(&log.output).map_err(|error| GitFailure::failed(&error.to_string()))?
    } else {
        Vec::new()
    };
    for commit in commits {
        let matches =
            commit.subject.to_lowercase().contains(&query) || commit.oid.starts_with(&query);
        if matches {
            let label = format!("{} {}", commit.short, commit.subject);
            let token = format!("@{}", commit.short);
            entries.push(entry(
                &Named::Commit(commit.oid.clone()),
                &label,
                &token,
                COMMIT,
            ));
        }
    }
    let incomplete = entries.len() > limit;
    entries.truncate(limit);
    Ok(ProviderSuggestions::new(entries, incomplete))
}

fn entry(named: &Named, label: &str, token: &str, variant: &str) -> MentionEntry {
    MentionEntry {
        key: named.key(),
        label: label.to_owned(),
        token: token.to_owned(),
        variant: variant.to_owned(),
        warning: None,
        expands: false,
    }
}

/// What a mention gives, read when its message is sent.
#[derive(Debug)]
struct Mentioned {
    text: String,
    label: String,
    variant: String,
    summary: MentionSummary,
}

impl Mentioned {
    /// A mention whose reads would ask the owner, so it gives a note.
    fn not_read(named: &Named) -> Self {
        let (what, label, variant) = match named {
            Named::Changes => (
                "the uncommitted changes".to_owned(),
                "Uncommitted changes".to_owned(),
                CHANGES,
            ),
            Named::Commit(oid) => (
                format!("the commit {oid}"),
                oid.chars().take(7).collect(),
                COMMIT,
            ),
            Named::Branch(name) => (format!("the branch {name}"), name.clone(), BRANCH),
        };
        Self {
            text: texts::mention_not_read(&what),
            label,
            variant: variant.to_owned(),
            summary: summary(NOT_READ, &[]),
        }
    }
}

impl MentionContent for Mentioned {
    fn render(&self, limit: usize) -> RenderedMention {
        RenderedMention::new(
            cut(&self.text, limit),
            self.label.clone(),
            self.variant.clone(),
            self.summary.clone(),
        )
    }
}

fn summary(code: &str, values: &[(&str, usize)]) -> MentionSummary {
    MentionSummary {
        code: code.to_owned(),
        values: values
            .iter()
            .map(|(name, value)| {
                (
                    (*name).to_owned(),
                    u32::try_from(*value).unwrap_or(u32::MAX),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    }
}

async fn read_named(reading: &Reading, named: &Named) -> Result<Mentioned, GitFailure> {
    match named {
        Named::Changes => read_changes(reading).await,
        Named::Commit(oid) => read_commit(reading, oid).await,
        Named::Branch(name) => read_branch(reading, name).await,
    }
}

/// The status, then the staged and the unstaged patches.
async fn read_changes(reading: &Reading) -> Result<Mentioned, GitFailure> {
    let status = read_status(reading).await?;
    let mut text = texts::mentioned_changes(&reading.layout.top.display().to_string());
    text.push_str(&status_text(&status, None, None));
    for (heading, cached) in [("Staged changes", true), ("Unstaged changes", false)] {
        let mut command = vec!["diff".to_owned()];
        if cached {
            command.push("--cached".to_owned());
        }
        command.extend(PATCH_OPTIONS.iter().map(|option| (*option).to_owned()));
        command.push("--ignore-submodules=dirty".to_owned());
        command.push("--patch".to_owned());
        let patch = reading
            .git
            .output(
                reading
                    .run(command)
                    .with_output_limit(output_limit(reading)),
            )
            .await?;
        let patch = patch.text();
        if !patch.is_empty() {
            let _written = write!(text, "\n{heading}:\n{patch}");
        }
    }
    Ok(Mentioned {
        text,
        label: "Uncommitted changes".to_owned(),
        variant: CHANGES.to_owned(),
        summary: summary(CHANGES, &[("files", status.entries.len())]),
    })
}

/// The commit with its message, its files and its patch.
async fn read_commit(reading: &Reading, oid: &str) -> Result<Mentioned, GitFailure> {
    let commit = reading.commit("commit", oid).await?;
    let shown = shown_commit(reading, &commit, true).await?;
    let mut text = texts::mentioned_commit(&shown.details.short);
    text.push_str(&shown.text);
    let subject = shown.details.message.lines().next().unwrap_or_default();
    Ok(Mentioned {
        text,
        label: format!("{} {subject}", shown.details.short),
        variant: COMMIT.to_owned(),
        summary: summary(COMMIT, &[("files", shown.files.len())]),
    })
}

/// The branch's commits since it left its upstream or the default branch,
/// and the files that they change.
async fn read_branch(reading: &Reading, name: &str) -> Result<Mentioned, GitFailure> {
    let full_name = format!("refs/heads/{name}");
    let tip = reading
        .commit("branch", &full_name)
        .await
        .map_err(|_unknown| GitFailure::UnknownRevision {
            revision: name.to_owned(),
        })?;
    let base = branch_base(reading, &full_name, &tip).await?;
    let range = base
        .as_ref()
        .map_or_else(|| tip.clone(), |(_name, oid)| format!("{oid}..{tip}"));
    let log = reading
        .git
        .output(reading.run([
            "log".to_owned(),
            "-z".to_owned(),
            LOG_FORMAT.to_owned(),
            format!("--max-count={BRANCH_COMMITS}"),
            range,
        ]))
        .await?;
    let commits = parse_log(&log.output).map_err(|error| GitFailure::failed(&error.to_string()))?;
    let mut text = match &base {
        Some((base_name, _oid)) => texts::mentioned_branch_since(name, base_name, commits.len()),
        None => texts::mentioned_branch(name, commits.len()),
    };
    for commit in &commits {
        let _written = writeln!(
            text,
            "{} {} {}: {}",
            commit.short,
            commit.day(),
            commit.author,
            commit.subject
        );
    }
    let mut files = 0;
    if let Some((_name, oid)) = &base {
        let mut command = vec!["diff".to_owned()];
        command.extend(PATCH_OPTIONS.iter().map(|option| (*option).to_owned()));
        command.extend([
            "--numstat".to_owned(),
            "-z".to_owned(),
            format!("{oid}...{tip}"),
        ]);
        let numstat = reading.git.output(reading.run(command)).await?;
        let stats = parse_numstat(&numstat.output);
        files = stats.len();
        let _written = writeln!(text, "\nFiles that the branch changes ({files}):");
        for stat in &stats {
            let _written = writeln!(text, "  {}", stat_line(stat));
        }
    }
    Ok(Mentioned {
        text,
        label: name.to_owned(),
        variant: BRANCH.to_owned(),
        summary: summary(BRANCH, &[("commits", commits.len()), ("files", files)]),
    })
}

/// What a branch is compared with: its upstream, or else the default
/// branch, `origin`'s HEAD, `main` or `master`, when it is another branch
/// with commits in common. The base's name and its commit.
async fn branch_base(
    reading: &Reading,
    full_name: &str,
    tip: &str,
) -> Result<Option<(String, String)>, GitFailure> {
    let upstream = reading
        .git
        .output(reading.run(["for-each-ref", "--format=%(upstream:short)", full_name]))
        .await?;
    let mut candidates: Vec<String> = upstream
        .text()
        .lines()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    candidates.extend(["origin/HEAD", "main", "master"].map(str::to_owned));
    for candidate in candidates {
        let Ok(oid) = reading.commit("base", &candidate).await else {
            continue;
        };
        if oid == tip {
            continue;
        }
        let merge_base = reading
            .git
            .run(reading.run(["merge-base", oid.as_str(), tip]))
            .await?;
        if merge_base.succeeded() {
            return Ok(Some((candidate, merge_base.text().trim().to_owned())));
        }
    }
    Ok(None)
}

/// The output that a mention keeps of one run, at least 64 KiB.
fn output_limit(reading: &Reading) -> u32 {
    u32::try_from(reading.room.max(64 * 1024)).unwrap_or(u32::MAX)
}

/// At most `limit` bytes of `text`, cut at a character.
fn cut(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_git_testing::{HostGit, TestRepository};
    use mannie_plugin_sdk::BoxFuture;
    use mannie_plugin_sdk::contracts::ConversationId;
    use mannie_plugin_sdk::points::mentions::{
        MentionContext, MentionProvider, ReadCheck, ReadVerdict,
    };
    use mannie_plugin_sdk::services::processes::ReadPlace;

    use super::{GitMentions, Named, cut, is_branch_name};
    use crate::git::Git;

    /// The core's check of reads, which these tests do not reach: the
    /// provider checks its runs through the service instead.
    #[derive(Debug)]
    struct AllowEverything;

    impl ReadCheck for AllowEverything {
        fn can_read(&self, _key: &str) -> BoxFuture<'static, ReadVerdict> {
            Box::pin(std::future::ready(ReadVerdict::Allowed))
        }
    }

    fn context(repository: &TestRepository) -> MentionContext {
        let conversation = ConversationId::parse("0199e0f2-0000-7000-8000-000000000003").unwrap();
        MentionContext::new(
            Some(conversation),
            None,
            Some(repository.path().to_path_buf()),
            false,
            Arc::new(AllowEverything),
        )
    }

    fn repository() -> TestRepository {
        let repository = TestRepository::new();
        repository.write("a.txt", "one\n");
        repository.commit("Start");
        repository.git(&["switch", "--quiet", "-c", "feature"]);
        repository.write("b.txt", "feature\n");
        repository.commit("Add the parser");
        repository.write("a.txt", "two\n");
        repository
    }

    #[tokio::test]
    async fn the_list_offers_the_changes_the_branches_and_the_commits_that_match() {
        let repository = repository();
        let host = Arc::new(HostGit::new());
        let provider = GitMentions::new(Git::new(Arc::<HostGit>::clone(&host)));
        let context = context(&repository);

        let all = provider.suggest(&context, "", 10).await.unwrap();
        let parser = provider.suggest(&context, "PARSER", 10).await.unwrap();
        let first = provider.suggest(&context, "", 1).await.unwrap();

        let keys: Vec<&str> = all.entries.iter().map(|entry| entry.key.as_str()).collect();
        assert_eq!(keys[..3], ["changes", "branch:feature", "branch:main"]);
        assert_eq!(all.entries.len(), 5);
        assert!(
            all.entries[3].label.ends_with(" Add the parser"),
            "{:?}",
            all.entries[3]
        );
        assert_eq!(parser.entries.len(), 1);
        assert_eq!(parser.entries[0].variant, "commit");
        assert!(first.incomplete);
        assert!(provider.offered(&context));
        // Every run is checked as the conversation's reads of the repository.
        for run in host
            .runs()
            .iter()
            .filter(|run| run.arguments != ["version"])
        {
            assert!(
                matches!(&run.reads, Some(ReadPlace::Conversation { folders, .. }) if !folders.is_empty()),
                "{run:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_mention_gives_the_changes_a_commit_or_a_branch() {
        let repository = repository();
        let provider = GitMentions::new(Git::new(Arc::new(HostGit::new())));
        let context = context(&repository);
        let head = repository.head();

        let changes = provider
            .resolve(&context, "changes", 100_000)
            .await
            .unwrap()
            .render(100_000);
        let commit = provider
            .resolve(&context, &format!("commit:{head}"), 100_000)
            .await
            .unwrap()
            .render(100_000);
        let branch = provider
            .resolve(&context, "branch:feature", 100_000)
            .await
            .unwrap()
            .render(100_000);
        let short = provider
            .resolve(&context, "changes", 100_000)
            .await
            .unwrap()
            .render(40);

        assert!(
            changes
                .text
                .contains("Not staged (1):\n  modified: a.txt\n"),
            "{}",
            changes.text
        );
        assert!(
            changes
                .text
                .contains("Unstaged changes:\ndiff --git a/a.txt"),
            "{}",
            changes.text
        );
        assert_eq!(changes.summary.code, "changes");
        assert!(
            commit.text.contains("    Add the parser\n"),
            "{}",
            commit.text
        );
        assert!(commit.text.contains("+feature\n"), "{}", commit.text);
        assert!(
            branch.text.contains("since it left main:\n"),
            "{}",
            branch.text
        );
        assert!(
            branch
                .text
                .contains("Files that the branch changes (1):\n  b.txt +1 -0\n"),
            "{}",
            branch.text
        );
        assert_eq!(branch.summary.values["commits"], 1);
        assert!(short.text.len() <= 40);
    }

    #[tokio::test]
    async fn a_mention_that_would_need_permission_gives_a_note_and_bad_keys_are_refused() {
        let repository = repository();
        let refusing = GitMentions::new(Git::new(Arc::new(HostGit::refusing_reads())));
        let provider = GitMentions::new(Git::new(Arc::new(HostGit::new())));
        let context = context(&repository);

        let note = refusing
            .resolve(&context, "changes", 10_000)
            .await
            .unwrap()
            .render(10_000);
        let invalid = provider
            .resolve(&context, "commit:--output=x", 10_000)
            .await
            .err();
        let unknown = provider
            .resolve(&context, "branch:nothing", 10_000)
            .await
            .err();
        let listed = refusing.suggest(&context, "", 10).await.err();

        assert!(
            note.text.contains("needs the owner's permission"),
            "{}",
            note.text
        );
        assert_eq!(note.summary.code, "not_read");
        assert_eq!(invalid.as_deref(), Some("invalid_key"));
        assert_eq!(unknown.as_deref(), Some("unknown_revision"));
        assert_eq!(listed.as_deref(), Some("read_not_allowed"));
    }

    #[test]
    fn keys_are_checked_again_before_git_runs() {
        let oid = "1".repeat(40);

        assert_eq!(Named::parse("changes"), Some(Named::Changes));
        assert_eq!(
            Named::parse(&format!("commit:{oid}")),
            Some(Named::Commit(oid))
        );
        assert_eq!(
            Named::parse("branch:feature/parser"),
            Some(Named::Branch("feature/parser".to_owned()))
        );
        for refused in [
            "commit:1a2b3c4",
            "commit:--output=x",
            "branch:-f",
            "branch:main..other",
            "branch:main@{1}",
            "branch:a b",
            "branch:",
            "tag:v1",
        ] {
            assert_eq!(Named::parse(refused), None, "{refused}");
        }
        assert!(is_branch_name("release-1.0"));
        assert_eq!(cut("héllo", 2), "h");
    }
}
