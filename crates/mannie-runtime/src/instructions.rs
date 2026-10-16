//! Instruction sections (docs/architecture/extension-points.md, instruction
//! sections): the system prompt is assembled from sections that modules
//! register, each at a fixed position. Stable sections come first, so that
//! providers can cache the prefix; the checkpoint, which changes more often,
//! comes after them.
//!
//! A composition root can assemble the prompt from sections of its own, as
//! the worker does for the tasks that the master assigns (ADR-0028, decision
//! 3); its sections read what an assigned run works under from the context.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use mannie_types::ceiling::CeilingMode;
use mannie_types::projects::ProjectView;

use crate::agents::AgentDefinition;
use crate::texts;

/// What the sections of one call can draw on.
#[derive(Debug, Clone, Copy)]
pub struct InstructionContext<'a> {
    /// The agent that makes the call.
    pub agent: &'a AgentDefinition,
    /// The owner's global instructions, possibly empty.
    pub owner_instructions: &'a str,
    /// The summary of the checkpoint that stands in for older turns.
    pub checkpoint: Option<&'a str>,
    /// Where the run's tools work, or `None` for a run without tools.
    pub tools: Option<ToolEnvironment<'a>>,
    /// Whether the run is offered the task tools, which give tasks to child
    /// agents.
    pub offers_tasks: bool,
    /// For a run of an assigned task on a machine with a ceiling, what it
    /// works under.
    pub machine: Option<MachineContext<'a>>,
}

/// What a run of an assigned task works under, for the sections of its
/// machine (ADR-0028).
#[derive(Debug, Clone, Copy)]
pub struct MachineContext<'a> {
    /// The mode of the task's ceiling, narrowed as the task asks.
    pub mode: CeilingMode,
    /// The ceiling's allowed folders; the first is the workspace.
    pub allowed_folders: &'a [PathBuf],
    /// The machine's instructions, as its file said when the run started.
    pub instructions: Option<&'a str>,
}

/// Where a run's tools work, for the environment section.
#[derive(Debug, Clone, Copy)]
pub struct ToolEnvironment<'a> {
    /// The operating system with its version and architecture.
    pub system: &'a str,
    /// The shell that runs commands, if there is one.
    pub shell: Option<&'a str>,
    /// The conversation's project, whose folder is its workspace, or `None`
    /// for a general conversation.
    pub project: Option<&'a ProjectView>,
    /// The owner's projects, the oldest first, which a general conversation
    /// can move into; empty for a run that cannot move.
    pub projects: &'a [ProjectView],
    /// Whether the conversation, or the parent of a child's, is in full
    /// access, so its operations run without the owner's approval
    /// (ADR-0029).
    pub full_access: bool,
}

/// The text of a section for a call, or `None` to leave it out.
pub type SectionText = Arc<dyn Fn(&InstructionContext<'_>) -> Option<String> + Send + Sync>;

/// One section of the system prompt.
#[derive(Clone)]
pub struct InstructionSection {
    /// Unique among sections.
    pub id: &'static str,
    /// Sections are ordered by this number, lowest first.
    pub position: u32,
    /// The section's text for a call. A section can hold what it needs, such
    /// as what a composition root knows of its machine.
    pub text: SectionText,
}

impl InstructionSection {
    /// A section with this text.
    pub fn new(
        id: &'static str,
        position: u32,
        text: impl Fn(&InstructionContext<'_>) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            id,
            position,
            text: Arc::new(text),
        }
    }
}

impl fmt::Debug for InstructionSection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstructionSection")
            .field("id", &self.id)
            .field("position", &self.position)
            .finish_non_exhaustive()
    }
}

/// A section whose ID is already registered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the instruction section {id} is already registered")]
pub struct DuplicateSection {
    /// The ID.
    pub id: &'static str,
}

/// The registered sections.
#[derive(Debug, Clone, Default)]
pub struct InstructionRegistry {
    sections: Vec<InstructionSection>,
}

/// Mannie's sections: the base instructions for whom the agent answers to,
/// the agent's instructions, the guidance on tools and on tasks, the owner's
/// instructions, where the tools work, and the checkpoint. The sections of
/// tools appear only in runs with tools, and the guidance on tasks only in
/// runs that are offered the task tools. The executors of plugins add their
/// sections to the runs that are offered their tools (ADR-0031).
pub fn builtin_sections() -> Vec<InstructionSection> {
    vec![
        InstructionSection::new("base", 0, |context| {
            Some(texts::base_instructions(context.agent.requester).to_owned())
        }),
        InstructionSection::new("agent", 100, |context| {
            non_empty(context.agent.instructions).map(str::to_owned)
        }),
        InstructionSection::new("tools", 150, |context| {
            context.tools.map(|tools| {
                if tools.full_access {
                    texts::FULL_ACCESS_TOOL_GUIDANCE.to_owned()
                } else {
                    texts::TOOL_GUIDANCE.to_owned()
                }
            })
        }),
        InstructionSection::new("tasks", 160, |context| {
            context
                .offers_tasks
                .then(|| texts::TASKS_GUIDANCE.to_owned())
        }),
        InstructionSection::new("owner", 200, |context| {
            non_empty(context.owner_instructions).map(|instructions| {
                format!(
                    "{}\n\n{}",
                    texts::OWNER_INSTRUCTIONS_HEADING,
                    instructions.trim()
                )
            })
        }),
        InstructionSection::new("environment", 300, |context| {
            context.tools.map(|tools| texts::environment(&tools))
        }),
        checkpoint_section(),
    ]
}

/// The section of the checkpoint, the last of every prompt.
pub fn checkpoint_section() -> InstructionSection {
    InstructionSection::new("checkpoint", 1_000, |context| {
        context
            .checkpoint
            .and_then(non_empty)
            .map(|summary| format!("{}\n\n{}", texts::CHECKPOINT_HEADING, summary.trim()))
    })
}

fn non_empty(text: &str) -> Option<&str> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

impl InstructionRegistry {
    /// A registry without sections.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with Mannie's sections.
    pub fn with_builtin() -> Self {
        Self {
            sections: builtin_sections(),
        }
    }

    /// Registers a section. IDs are unique.
    pub fn register(&mut self, section: InstructionSection) -> Result<(), DuplicateSection> {
        if self
            .sections
            .iter()
            .any(|existing| existing.id == section.id)
        {
            return Err(DuplicateSection { id: section.id });
        }
        self.sections.push(section);
        Ok(())
    }

    /// Whether a section with this ID is registered.
    pub fn contains(&self, id: &str) -> bool {
        self.sections.iter().any(|section| section.id == id)
    }

    /// The system prompt for a call: the sections in their order, separated by
    /// blank lines.
    pub fn assemble(&self, context: &InstructionContext<'_>) -> String {
        self.assemble_with(context, &[])
    }

    /// The system prompt for a call with the registered sections and the
    /// sections of one run, such as those of the plugins that were on when
    /// it started, in their order.
    pub fn assemble_with(
        &self,
        context: &InstructionContext<'_>,
        extra: &[InstructionSection],
    ) -> String {
        let mut sections: Vec<&InstructionSection> = self.sections.iter().chain(extra).collect();
        sections.sort_by_key(|section| section.position);
        sections
            .into_iter()
            .filter_map(|section| (section.text)(context))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use mannie_types::clock::UtcMillis;
    use mannie_types::ids::ProjectId;
    use mannie_types::projects::ProjectView;

    use super::{
        DuplicateSection, InstructionContext, InstructionRegistry, InstructionSection,
        ToolEnvironment,
    };
    use crate::agents::{CONVERSATION, EXPLORE};
    use crate::texts;

    fn project(number: u32, name: &str, folder: &str) -> ProjectView {
        ProjectView {
            id: ProjectId::parse(&format!("0199e0f2-0000-7000-a000-{number:012}")).unwrap(),
            name: name.to_owned(),
            folder: folder.to_owned(),
            created_at: UtcMillis::EPOCH,
        }
    }

    #[test]
    fn sections_join_in_their_order_and_empty_ones_are_left_out() {
        let registry = InstructionRegistry::with_builtin();
        let plain = InstructionContext {
            agent: &CONVERSATION,
            owner_instructions: "  ",
            checkpoint: None,
            tools: None,
            offers_tasks: false,
            machine: None,
        };
        let full = InstructionContext {
            agent: &CONVERSATION,
            owner_instructions: "Answer in German.\n",
            checkpoint: Some("They talked about tea."),
            tools: None,
            offers_tasks: false,
            machine: None,
        };

        assert_eq!(registry.assemble(&plain), texts::BASE_INSTRUCTIONS);
        assert_eq!(
            registry.assemble(&full),
            format!(
                "{}\n\n{}\n\nAnswer in German.\n\n{}\n\nThey talked about tea.",
                texts::BASE_INSTRUCTIONS,
                texts::OWNER_INSTRUCTIONS_HEADING,
                texts::CHECKPOINT_HEADING
            )
        );
    }

    #[test]
    fn a_registered_section_takes_its_position_and_ids_are_unique() {
        let mut registry = InstructionRegistry::with_builtin();
        let fake = InstructionSection::new("fake", 150, |_context| Some("FAKE".to_owned()));
        registry.register(fake.clone()).unwrap();
        let context = InstructionContext {
            agent: &CONVERSATION,
            owner_instructions: "Owner text.",
            checkpoint: None,
            tools: None,
            offers_tasks: false,
            machine: None,
        };

        let assembled = registry.assemble(&context);
        let duplicate = registry.register(fake);

        let fake_at = assembled.find("FAKE").unwrap();
        let owner_at = assembled.find("Owner text.").unwrap();
        assert!(fake_at < owner_at, "{assembled}");
        assert_eq!(duplicate, Err(DuplicateSection { id: "fake" }));
    }

    #[test]
    fn runs_with_tools_get_the_guidance_and_their_environment() {
        let registry = InstructionRegistry::with_builtin();
        let work = project(1, "work", "/work");
        let projects = [work.clone()];
        let context = InstructionContext {
            agent: &CONVERSATION,
            owner_instructions: "Owner text.",
            checkpoint: None,
            tools: Some(ToolEnvironment {
                system: "Linux 6.8, x86_64",
                shell: Some("sh (dash)"),
                project: Some(&work),
                projects: &projects,
                full_access: false,
            }),
            offers_tasks: false,
            machine: None,
        };

        let assembled = registry.assemble(&context);

        let guidance_at = assembled.find(texts::TOOL_GUIDANCE).unwrap();
        let owner_at = assembled.find("Owner text.").unwrap();
        let environment_at = assembled
            .find("- Workspace: /work, the folder of the project work")
            .unwrap();
        assert!(
            guidance_at < owner_at && owner_at < environment_at,
            "{assembled}"
        );
        assert!(
            assembled.contains("- Shell for run_command: sh (dash)"),
            "{assembled}"
        );
        assert!(!assembled.contains("move_to_project"), "{assembled}");
    }

    #[test]
    fn a_child_reads_its_own_base_and_only_a_parent_reads_the_guidance_on_tasks() {
        let registry = InstructionRegistry::with_builtin();
        let tools = ToolEnvironment {
            system: "Linux 6.8, x86_64",
            shell: None,
            project: None,
            projects: &[],
            full_access: false,
        };
        let parent = InstructionContext {
            agent: &CONVERSATION,
            owner_instructions: "Owner text.",
            checkpoint: None,
            tools: Some(tools),
            offers_tasks: true,
            machine: None,
        };
        let child = InstructionContext {
            agent: &EXPLORE,
            offers_tasks: false,
            ..parent
        };

        let parent = registry.assemble(&parent);
        let child = registry.assemble(&child);

        assert!(parent.starts_with(texts::BASE_INSTRUCTIONS), "{parent}");
        let guidance_at = parent.find(texts::TOOL_GUIDANCE).unwrap();
        let tasks_at = parent.find(texts::TASKS_GUIDANCE).unwrap();
        let owner_at = parent.find("Owner text.").unwrap();
        assert!(guidance_at < tasks_at && tasks_at < owner_at, "{parent}");
        assert!(child.starts_with(texts::CHILD_BASE_INSTRUCTIONS), "{child}");
        assert!(child.contains(texts::EXPLORE_INSTRUCTIONS), "{child}");
        assert!(!child.contains(texts::TASKS_GUIDANCE), "{child}");
        assert!(!child.contains(texts::BASE_INSTRUCTIONS), "{child}");
    }

    #[test]
    fn a_general_conversation_learns_the_projects_it_can_move_into() {
        let projects = [
            project(1, "older", "/work/older"),
            project(2, "mannie", "/work/mannie"),
        ];
        let general = |projects| ToolEnvironment {
            system: "Linux 6.8, x86_64",
            shell: None,
            project: None,
            projects,
            full_access: false,
        };

        let listed = texts::environment(&general(&projects));
        let alone = texts::environment(&general(&[]));

        assert!(
            listed.contains("- Workspace: none, because this conversation belongs to no project"),
            "{listed}"
        );
        let newest_at = listed.find("  - mannie: /work/mannie").unwrap();
        let older_at = listed.find("  - older: /work/older").unwrap();
        assert!(newest_at < older_at, "{listed}");
        assert!(listed.contains("call move_to_project first"), "{listed}");
        assert!(!alone.contains("move_to_project"), "{alone}");
        assert!(alone.contains("- Shell for run_command: none"), "{alone}");
    }

    #[test]
    fn the_environment_lists_a_limited_number_of_projects() {
        let projects: Vec<ProjectView> = (1..=60)
            .map(|number| {
                project(
                    number,
                    &format!("project {number}"),
                    &format!("/work/{number}"),
                )
            })
            .collect();

        let listed = texts::environment(&ToolEnvironment {
            system: "Linux 6.8, x86_64",
            shell: None,
            project: None,
            projects: &projects,
            full_access: false,
        });

        assert!(listed.contains("  - project 60: /work/60"), "{listed}");
        assert!(!listed.contains("  - project 10: /work/10"), "{listed}");
        assert!(listed.contains("  - and 10 more"), "{listed}");
    }
}
