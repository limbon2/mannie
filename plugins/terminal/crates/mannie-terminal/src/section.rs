//! The section `terminals` of the system prompt (plugins/terminal/docs/
//! README.md, what the model knows): when terminals are right, that the
//! model reads only its own commands and what the owner mentions, how to
//! cite output, and where new terminals open. It is rendered for each model
//! call of a run that is offered `run_in_terminal`, so it follows the hosts.
//!
//! The owner named the hosts, and their names are cleaned as the core cleans
//! labels.

use mannie_plugin_sdk::contracts::clean_label;
use mannie_plugin_sdk::points::sections::{SectionContext, SectionSource};
use mannie_plugin_sdk::services::terminals::TerminalHostState;

use crate::registry::Terminals;
use crate::texts;
use crate::tools::RUN_IN_TERMINAL;

/// The section's ID.
pub(crate) const SECTION_ID: &str = "terminals";

/// The section's place, after the SSH hosts (320) and Git (330).
pub(crate) const SECTION_POSITION: u32 = 340;

/// The agents whose runs are offered the tools and read the section: the
/// owner's conversations only (D45).
pub(crate) const AGENTS: [&str; 1] = ["conversation"];

/// The most characters of a host's name in the section.
const MAX_NAME_CHARS: usize = 64;

/// The terminals' section.
#[derive(Debug)]
pub(crate) struct TerminalsSection {
    terminals: Terminals,
}

impl TerminalsSection {
    pub(crate) fn new(terminals: Terminals) -> Self {
        Self { terminals }
    }
}

impl SectionSource for TerminalsSection {
    fn render(&self, context: &SectionContext) -> Option<String> {
        let is_offered = context
            .offered_tools
            .iter()
            .any(|tool| tool == RUN_IN_TERMINAL);
        if !is_offered {
            return None;
        }
        let hosts = self.terminals.hosts();
        let shells: Vec<String> = hosts
            .shells
            .iter()
            .filter_map(|shell| clean_label(&shell.name, MAX_NAME_CHARS))
            .collect();
        let remote: Vec<String> = hosts
            .hosts
            .iter()
            .filter(|host| matches!(host.state, TerminalHostState::Ready))
            .filter_map(|host| clean_label(&host.label, MAX_NAME_CHARS))
            .collect();
        Some(format!(
            "{}\n{}",
            texts::SECTION,
            texts::section_hosts(&shells, &remote)
        ))
    }
}
