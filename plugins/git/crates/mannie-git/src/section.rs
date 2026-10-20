//! The section `git` of the system prompt (plugins/git/docs/tools.md, the
//! text the model sees): how to use the git tools instead of running git
//! with `run_command`, and that what they return is data. It reaches only
//! the calls that are offered the git tools, which the core offers where
//! the workspace is in a repository (ADR-0038, decisions 1 and 2).

use mannie_plugin_sdk::points::sections::{SectionContext, SectionSource};

use crate::texts;
use crate::tools::commit::GIT_COMMIT;
use crate::tools::status::GIT_STATUS;

/// The section's ID.
pub(crate) const SECTION_ID: &str = "git";

/// The section's place, after the core's environment (300) and the SSH
/// hosts (320).
pub(crate) const SECTION_POSITION: u32 = 330;

/// The section, for the calls that have the git tools.
#[derive(Debug)]
pub(crate) struct GitSection;

impl SectionSource for GitSection {
    fn render(&self, context: &SectionContext) -> Option<String> {
        if !context.offers_tool(GIT_STATUS) {
            return None;
        }
        // The explore agent reads only, so it hears nothing of changes.
        if context.offers_tool(GIT_COMMIT) {
            Some(format!(
                "{}\n{}",
                texts::SECTION_READS,
                texts::SECTION_CHANGES
            ))
        } else {
            Some(texts::SECTION_READS.to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::points::sections::{SectionContext, SectionSource};
    use serde_json::json;

    use super::GitSection;
    use crate::texts;

    fn context(offered_tools: &[&str]) -> SectionContext {
        serde_json::from_value(json!({
            "agent": "conversation",
            "requester": "owner",
            "usesTools": true,
            "offersTasks": false,
            "fullAccess": false,
            "offeredTools": offered_tools,
        }))
        .unwrap()
    }

    #[test]
    fn the_section_reaches_only_the_calls_that_have_the_git_tools() {
        assert_eq!(
            GitSection
                .render(&context(&["read_file", "git_status"]))
                .as_deref(),
            Some(texts::SECTION_READS)
        );
        let changes = GitSection
            .render(&context(&["git_status", "git_commit"]))
            .unwrap();
        assert!(changes.ends_with(texts::SECTION_CHANGES), "{changes}");
        assert_eq!(GitSection.render(&context(&["read_file"])), None);
    }
}
