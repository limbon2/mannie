//! The section `workers-databases` of the system prompt (plugins/workers/
//! docs/databases.md, agents): every registered database with its file, its
//! machine, whether it is read-only and whether it can run statements, for
//! the runs that are offered the database tools. It is rendered for each
//! model call from the feature's list, so it follows the databases as they
//! change.
//!
//! The owner chose the names, and the files' names come from the owner's
//! disk, so the section introduces them as data and cleans each as the core
//! cleans labels.

use mannie_plugin_sdk::contracts::clean_label;
use mannie_plugin_sdk::points::sections::{SectionContext, SectionSource};

use super::commands::DatabaseView;
use super::registry::Databases;
use super::texts;
use super::tools::{CHANGING_AGENTS, READING_AGENTS};

/// The section's ID.
pub(crate) const SECTION_ID: &str = "workers-databases";

/// The section's place, after the core's environment (300), the task
/// executors' sections and the SSH hosts (320).
pub(crate) const SECTION_POSITION: u32 = 330;

/// The most characters of a name or a file's name in the section.
const MAX_NAME_CHARS: usize = 200;

/// The databases, as the model reads them.
#[derive(Debug)]
pub(crate) struct DatabasesSection {
    databases: Databases,
}

impl DatabasesSection {
    pub(crate) fn new(databases: Databases) -> Self {
        Self { databases }
    }
}

impl SectionSource for DatabasesSection {
    fn render(&self, context: &SectionContext) -> Option<String> {
        let agent = context.agent.as_str();
        let is_offered = context.uses_tools && READING_AGENTS.contains(&agent);
        if !is_offered {
            return None;
        }
        let changes = CHANGING_AGENTS.contains(&agent);
        Some(render_databases(&self.databases.views(), changes))
    }
}

/// The section for these databases, for an agent that may change them or
/// one that only reads.
pub(crate) fn render_databases(databases: &[DatabaseView], changes: bool) -> String {
    if databases.is_empty() {
        return texts::SECTION_EMPTY.to_owned();
    }
    let introduction = if changes {
        texts::SECTION_INTRODUCTION
    } else {
        texts::SECTION_INTRODUCTION_READING
    };
    let lines: Vec<String> = databases.iter().map(database_line).collect();
    format!("{introduction}\n{}", lines.join("\n"))
}

/// One database: its name, its file on its machine, its read-only mark and
/// its state.
fn database_line(database: &DatabaseView) -> String {
    let name = clean_label(&database.name, MAX_NAME_CHARS).unwrap_or_default();
    let file = clean_label(&database.file_name, MAX_NAME_CHARS).unwrap_or_default();
    let read_only = if database.read_only {
        ", read-only"
    } else {
        ""
    };
    format!(
        "- {name} (file {file} on this computer){read_only}{}",
        texts::state_note(database.state)
    )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::DatabaseKey;
    use mannie_plugin_sdk::services::sqlite_files::FileState;

    use super::render_databases;
    use crate::databases::commands::{DatabaseMachine, DatabaseView};
    use crate::databases::texts;

    fn database(name: &str, read_only: bool, state: FileState) -> DatabaseView {
        DatabaseView {
            key: DatabaseKey::parse("0199e0f2-0000-7000-8000-0000000000ee").unwrap(),
            name: name.to_owned(),
            file_name: format!("{}.db", name.to_lowercase()),
            read_only,
            state,
            machine: DatabaseMachine::ThisComputer,
        }
    }

    #[test]
    fn the_section_lists_each_database_with_its_file_mark_and_state() {
        let text = render_databases(
            &[
                database("Inventory", false, FileState::Available),
                database("Archive", true, FileState::Missing),
            ],
            true,
        );

        assert!(text.starts_with(texts::SECTION_INTRODUCTION));
        assert!(text.contains("- Inventory (file inventory.db on this computer)\n"));
        assert!(text.contains(
            "- Archive (file archive.db on this computer), read-only, not usable: its file is gone"
        ));
    }

    #[test]
    fn an_agent_that_only_reads_is_not_told_of_changes_and_no_databases_say_so() {
        let reading =
            render_databases(&[database("Inventory", false, FileState::Available)], false);

        assert!(reading.starts_with(texts::SECTION_INTRODUCTION_READING));
        assert!(!reading.contains("change_database"));
        assert_eq!(render_databases(&[], true), texts::SECTION_EMPTY);
    }
}
