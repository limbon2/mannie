//! Self-management provider for the git plugin (ADR-0081).
//!
//! Git stores no settings or credentials of its own; this provider exposes its
//! guide and read-only diagnostics so the conversation agent can explain how
//! git tools work and why git may be unavailable or refused in a workspace.

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementError, ManagementGuide, ManagementProvider,
    ManagementSnapshot, PrepareActionRequest, PreparedAction,
};
use serde_json::json;

use crate::git::Git;
use crate::version::Version;

/// Read-only management provider for the git plugin.
#[derive(Debug, Clone)]
pub(crate) struct GitManagement {
    git: Git,
}

impl GitManagement {
    /// Creates a management provider backed by the running git service.
    pub(crate) fn new(git: Git) -> Self {
        Self { git }
    }
}

impl ManagementProvider for GitManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Provides read and write git tools, repository mentions, and a git side panel \
                by running the owner's installed git without a shell through the core's processes \
                service. Tools are offered only when the conversation workspace is inside a git \
                repository (marked by a .git directory or file)."
                .to_owned(),
            storage: "Stores no SQLite tables, files, or credentials in Mannie. Remote \
                authentication and commit signing use the owner's existing git configuration, \
                credential helpers, SSH agent, and GnuPG home inherited by the manifest."
                .to_owned(),
            troubleshooting: "Requires git 2.40 or later on Mannie's PATH (git 2.54 or later adds \
                hook listing). Non-remote runs block remote protocols to avoid triggering promisor \
                remotes, and every run passes fixed flags that disable pagers, fsmonitor, auto-gc, \
                and submodule recursion. If git reports dubious ownership, check git safe.directory \
                on the host. If git reports a .lock file, check for another git process in that \
                repository."
                .to_owned(),
            actions: Vec::new(),
        }
    }

    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let cached = self.git.cached_version();

        Box::pin(async move {
            if let Some(query) = query
                && !query.trim().is_empty()
            {
                return Err(ManagementError::invalid_field(
                    "query",
                    format!("git does not support inspect query {query:?}"),
                ));
            }

            let version_text = cached.map(|version| version.to_string());
            let supports_hook_list = cached.map(|version| version >= Version::HOOK_LIST);
            let summary = match &version_text {
                Some(version) => format!("Git plugin is on (detected git {version})."),
                None => format!(
                    "Git plugin is on (git version not yet probed; requires {} or later).",
                    Version::MINIMUM
                ),
            };

            let data = json!({
                "minimumVersion": Version::MINIMUM.to_string(),
                "hookListMinimumVersion": Version::HOOK_LIST.to_string(),
                "detectedVersion": version_text,
                "supportsHookList": supports_hook_list,
                "repositoryMarker": ".git",
                "storesCredentials": false,
                "storesTables": false,
            });

            Ok(ManagementSnapshot::new(summary, data))
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        Box::pin(std::future::ready(Err(ManagementError::unknown_action(
            &request.action,
        ))))
    }

    fn execute_action(
        &self,
        request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        Box::pin(std::future::ready(Err(ManagementError::unknown_action(
            &request.action,
        ))))
    }
}
