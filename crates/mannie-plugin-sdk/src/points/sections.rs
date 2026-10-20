//! Instruction sections (docs/architecture/extension-points.md, instruction
//! sections): a plugin's part of the system prompt, which the core places by
//! its position among the core's sections while the plugin is on.
//!
//! A section's text is fixed, or a source renders it for each call from what
//! the core tells it of the run: the agent, whether the run uses tools and
//! gives tasks, and full access. The source gets plain data and never a
//! handle into the core (ADR-0034, decision 3).
//!
//! A section's ID follows the rule `Global`: it is unique among the core's
//! sections and every plugin's (ADR-0034, decision 6).

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::contributions::{ExtensionPoint, IdRule};
use crate::points::agents::Requester;

/// The point of plugins' instruction sections, whose entries are
/// [`Section`]s named by their IDs.
#[derive(Debug)]
pub enum Sections {}

impl ExtensionPoint for Sections {
    const NAME: &'static str = "sections";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Global;
    type Entry = Section;

    fn id(entry: &Section) -> &str {
        entry.id
    }
}

/// One section of the system prompt.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Section {
    /// Unique among the core's sections and every plugin's.
    pub id: &'static str,
    /// Sections are ordered by this number, lowest first, among the core's
    /// sections and every plugin's.
    pub position: u32,
    /// The section's text.
    pub body: SectionBody,
}

impl Section {
    /// A section at a position, with its text.
    pub fn new(id: &'static str, position: u32, body: SectionBody) -> Self {
        Self { id, position, body }
    }
}

/// The text of a section.
#[derive(Debug, Clone)]
pub enum SectionBody {
    /// The same text in every call.
    Fixed(String),
    /// A text that the source renders for each call, or leaves out.
    Source(Arc<dyn SectionSource>),
}

/// Renders a section's text for a call from what the core tells it of the
/// run.
pub trait SectionSource: Send + Sync + fmt::Debug {
    /// The section's text for a call, or `None` to leave the section out of
    /// it.
    fn render(&self, context: &SectionContext) -> Option<String>;
}

/// What the core tells a section's source of the run that makes a call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SectionContext {
    /// The name of the agent that makes the call, such as `conversation`.
    pub agent: String,
    /// Whom the agent answers to.
    pub requester: Requester,
    /// Whether the run uses tools.
    pub uses_tools: bool,
    /// Whether the run is offered the task tools, which give tasks to child
    /// agents and to the plugins' executors.
    pub offers_tasks: bool,
    /// Whether the conversation, or the parent of a child's, is in full
    /// access, so the run's operations run without the owner's approval
    /// (ADR-0029). It is `false` for a run without tools, which has no
    /// operations.
    pub full_access: bool,
    /// The names of the tools that the call is offered, the core's and the
    /// plugins', so a plugin's section can speak only to calls that have its
    /// tools (ADR-0038, decision 1). It is empty for a run without tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offered_tools: Vec<String>,
}

impl SectionContext {
    /// The context of a call, which the core builds for each call of a
    /// source. A plugin's tests can decode one from JSON instead. The three
    /// switches are easy to swap, so callers pass named locals.
    #[cfg(feature = "host")]
    pub fn new(
        agent: String,
        requester: Requester,
        uses_tools: bool,
        offers_tasks: bool,
        full_access: bool,
    ) -> Self {
        Self {
            agent,
            requester,
            uses_tools,
            offers_tasks,
            full_access,
            offered_tools: Vec::new(),
        }
    }

    /// The same context, with the names of the tools that the call is
    /// offered.
    #[cfg(feature = "host")]
    #[must_use]
    pub fn with_offered_tools(mut self, offered_tools: Vec<String>) -> Self {
        self.offered_tools = offered_tools;
        self
    }

    /// Whether the call is offered the tool of this name.
    pub fn offers_tool(&self, name: &str) -> bool {
        self.offered_tools.iter().any(|offered| offered == name)
    }
}

#[cfg(test)]
mod tests {
    use super::{Section, SectionBody};

    #[test]
    fn the_constructor_keeps_each_argument_in_its_field() {
        let section = Section::new(
            "notes",
            400,
            SectionBody::Fixed("Notes are kept in the notes tool.".to_owned()),
        );

        assert_eq!(section.id, "notes");
        assert_eq!(section.position, 400);
        assert!(
            matches!(&section.body, SectionBody::Fixed(text) if text == "Notes are kept in the notes tool."),
            "{section:?}"
        );
    }

    #[cfg(feature = "host")]
    mod host {
        use std::sync::Arc;

        use crate::points::agents::Requester;
        use crate::points::sections::{SectionBody, SectionContext, SectionSource};

        /// A source that names the agent of a run with tools and leaves the
        /// section out of a run without them.
        #[derive(Debug)]
        struct ForToolRuns;

        impl SectionSource for ForToolRuns {
            fn render(&self, context: &SectionContext) -> Option<String> {
                context
                    .uses_tools
                    .then(|| format!("{} may use the notes tool.", context.agent))
            }
        }

        #[test]
        fn a_context_keeps_each_argument_in_its_field_and_a_source_renders_from_it() {
            let agent = "conversation".to_owned();
            let requester = Requester::Owner;
            let uses_tools = true;
            let offers_tasks = false;
            let full_access = true;

            let context =
                SectionContext::new(agent, requester, uses_tools, offers_tasks, full_access);
            let without_tools = SectionContext {
                uses_tools: false,
                ..context.clone()
            };
            let body = SectionBody::Source(Arc::new(ForToolRuns));

            assert_eq!(context.agent, "conversation");
            assert_eq!(context.requester, Requester::Owner);
            assert!(context.uses_tools);
            assert!(!context.offers_tasks);
            assert!(context.full_access);
            let SectionBody::Source(source) = body else {
                panic!("a source body");
            };
            assert_eq!(
                source.render(&context).as_deref(),
                Some("conversation may use the notes tool.")
            );
            assert_eq!(source.render(&without_tools), None);
        }
    }
}
