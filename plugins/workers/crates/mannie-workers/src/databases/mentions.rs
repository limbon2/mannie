//! The mention provider `workers.databases` (ADR-0032; plugins/workers/docs/
//! databases.md, agents): "@" in the message box lists the registered
//! databases by name, and a mention gives the model the database with its
//! structure, as far as it fits. It is offered in every conversation, since
//! every conversation can read every database.
//!
//! A mention reads the structure and no rows: the model reads rows only
//! through the database tools.

use std::collections::BTreeMap;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{DatabaseKey, MentionEntry, MentionSummary, clean_label};
use mannie_plugin_sdk::points::mentions::{
    MentionContent, MentionContext, MentionProvider, ProviderSuggestions, RenderedMention,
};

use super::commands::DatabaseView;
use super::output::structure_text;
use super::registry::{Databases, is_available};
use super::texts;

/// The provider's ID, which the plugin owns.
pub(crate) const PROVIDER_ID: &str = "workers.databases";

/// The variant of a database's entries, which chooses its icon.
pub(crate) const DATABASE_VARIANT: &str = "database";

/// The summary code of a mentioned database, which the UI words.
pub(crate) const DATABASE_SUMMARY: &str = "database";

/// The warning of a database that cannot run statements now, which the UI
/// words.
pub(crate) const NOT_USABLE_WARNING: &str = "not_usable";

/// The code of a key that names no registered database.
pub(crate) const DATABASE_NOT_FOUND: &str = "database_not_found";

/// The most characters of a name or a file's name in what the model reads.
const MAX_NAME_CHARS: usize = 200;

/// The databases, as "@" lists them.
#[derive(Debug)]
pub(crate) struct DatabaseMentions {
    databases: Databases,
}

impl DatabaseMentions {
    pub(crate) fn new(databases: Databases) -> Self {
        Self { databases }
    }
}

impl MentionProvider for DatabaseMentions {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn offered(&self, _context: &MentionContext) -> bool {
        true
    }

    fn suggest<'a>(
        &'a self,
        _context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        let suggestions = suggestions(&self.databases.views(), query, limit);
        Box::pin(std::future::ready(Ok(suggestions)))
    }

    fn resolve<'a>(
        &'a self,
        _context: &'a MentionContext,
        key: &'a str,
        _max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        Box::pin(async move {
            let database = DatabaseKey::parse(key)
                .ok()
                .and_then(|key| self.databases.by_key(&key))
                .ok_or_else(|| DATABASE_NOT_FOUND.to_owned())?;
            let structure = if is_available(&database) {
                self.databases.structure(database.key.clone()).await.ok()
            } else {
                None
            };
            let content: Box<dyn MentionContent> = Box::new(MentionedDatabase::of(
                &database,
                structure.map(|structure| structure_text(&structure)),
            ));
            Ok(content)
        })
    }
}

/// The databases whose names hold the query, those that start with it
/// first, then by name.
fn suggestions(databases: &[DatabaseView], query: &str, limit: usize) -> ProviderSuggestions {
    let query = query.to_lowercase();
    let mut matching: Vec<(bool, &DatabaseView)> = databases
        .iter()
        .filter_map(|database| {
            let name = database.name.to_lowercase();
            let starts = name.starts_with(&query);
            (starts || name.contains(&query)).then_some((starts, database))
        })
        .collect();
    matching.sort_by(|(left_starts, left), (right_starts, right)| {
        right_starts
            .cmp(left_starts)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    let incomplete = matching.len() > limit;
    let entries = matching
        .into_iter()
        .take(limit)
        .map(|(_starts, database)| MentionEntry {
            key: database.key.to_string(),
            label: database.name.clone(),
            token: format!("@{}", database.name),
            variant: DATABASE_VARIANT.to_owned(),
            warning: (!is_available(database)).then(|| NOT_USABLE_WARNING.to_owned()),
            expands: false,
        })
        .collect();
    ProviderSuggestions::new(entries, incomplete)
}

/// A database, as a sent mention keeps it.
#[derive(Debug)]
struct MentionedDatabase {
    name: String,
    text: String,
}

impl MentionedDatabase {
    fn of(database: &DatabaseView, structure: Option<String>) -> Self {
        let name = clean_label(&database.name, MAX_NAME_CHARS).unwrap_or_default();
        let file = clean_label(&database.file_name, MAX_NAME_CHARS).unwrap_or_default();
        let read_only = if database.read_only {
            ", read-only"
        } else {
            ""
        };
        let notes = format!(
            "{read_only}{}{}",
            texts::state_note(database.state),
            texts::reach_note(database.reach)
        );
        let machine = texts::machine(&database.machine);
        let text = match structure {
            Some(structure) => texts::mentioned(&name, &file, &machine, &notes, &structure),
            None => texts::mentioned_without_structure(&name, &file, &machine, &notes),
        };
        Self {
            name: database.name.clone(),
            text,
        }
    }
}

impl MentionContent for MentionedDatabase {
    fn render(&self, limit: usize) -> RenderedMention {
        RenderedMention::new(
            cut(&self.text, limit),
            self.name.clone(),
            DATABASE_VARIANT.to_owned(),
            MentionSummary {
                code: DATABASE_SUMMARY.to_owned(),
                values: BTreeMap::new(),
            },
        )
    }
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
    use mannie_plugin_sdk::contracts::DatabaseKey;
    use mannie_plugin_sdk::points::mentions::MentionContent;
    use mannie_plugin_sdk::services::sqlite_files::FileState;

    use super::{MentionedDatabase, NOT_USABLE_WARNING, suggestions};
    use crate::databases::commands::{DatabaseMachine, DatabaseReach, DatabaseView};

    fn database(name: &str, last: u8, state: FileState) -> DatabaseView {
        DatabaseView {
            key: DatabaseKey::parse(&format!("0199e0f2-0000-7000-8000-0000000000{last:02x}"))
                .unwrap(),
            name: name.to_owned(),
            file_name: format!("{}.db", name.to_lowercase()),
            read_only: false,
            state,
            machine: DatabaseMachine::ThisComputer,
            reach: DatabaseReach::Reachable,
        }
    }

    #[test]
    fn the_names_that_start_with_the_query_come_first_and_an_unusable_one_is_marked() {
        let databases = [
            database("Stock archive", 1, FileState::Missing),
            database("Inventory", 2, FileState::Available),
            database("Shop stock", 3, FileState::Available),
        ];

        let found = suggestions(&databases, "st", 10);

        let labels: Vec<&str> = found
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect();
        assert_eq!(labels, ["Stock archive", "Shop stock"]);
        assert_eq!(
            found.entries[0].warning.as_deref(),
            Some(NOT_USABLE_WARNING)
        );
        assert_eq!(found.entries[0].token, "@Stock archive");
        assert_eq!(found.entries[1].warning, None);
    }

    #[test]
    fn a_mention_gives_the_structure_within_its_limit() {
        let mentioned = MentionedDatabase::of(
            &database("Inventory", 2, FileState::Available),
            Some("parts (table):\n  id INTEGER, primary key 1\n".to_owned()),
        );

        let whole = mentioned.render(10_000);
        let short = mentioned.render(40);

        assert!(
            whole
                .text
                .contains("\"Inventory\" (file inventory.db on this computer)")
        );
        assert!(whole.text.contains("parts (table):"));
        assert!(short.text.len() <= 40);
    }
}
