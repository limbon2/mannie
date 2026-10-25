//! The mention provider `notebooks` (ADR-0050; plugins/notebooks/docs/
//! README.md, attaching): "@" in the message box lists the notebooks whose
//! names hold what the owner typed, those that start with it first, then the
//! most recently changed.
//!
//! The provider shares its ID with the context provider, so the core attaches
//! a mentioned notebook to the conversation through the context provider, and
//! the message gathers from it. The core never resolves such a mention.

use std::sync::Arc;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{ATTACHED_VARIANT, MentionEntry};
use mannie_plugin_sdk::points::mentions::{
    MentionContent, MentionContext, MentionProvider, ProviderSuggestions,
};

use crate::commands::NotebookView;
use crate::library::Library;
use crate::plugin::NOTEBOOKS;

/// The code of a mention that the core tried to resolve, which it never
/// does for a provider that shares its ID with a context provider.
pub(crate) const NOTEBOOK_ATTACHES: &str = "notebook_attaches";

/// The notebooks, as "@" lists them.
#[derive(Debug)]
pub(crate) struct NotebookMentions {
    library: Arc<Library>,
}

impl NotebookMentions {
    pub(crate) fn new(library: Arc<Library>) -> Self {
        Self { library }
    }
}

impl MentionProvider for NotebookMentions {
    fn id(&self) -> &str {
        NOTEBOOKS
    }

    fn offered(&self, _context: &MentionContext) -> bool {
        !self.library.list().notebooks.is_empty()
    }

    fn suggest<'a>(
        &'a self,
        _context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        let suggestions = suggestions(self.library.list().notebooks, query, limit);
        Box::pin(std::future::ready(Ok(suggestions)))
    }

    fn resolve<'a>(
        &'a self,
        _context: &'a MentionContext,
        _key: &'a str,
        _max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        Box::pin(std::future::ready(Err(NOTEBOOK_ATTACHES.to_owned())))
    }
}

/// The notebooks whose names hold the query, without regard to case: those
/// whose names start with it first, then the most recently changed.
fn suggestions(notebooks: Vec<NotebookView>, query: &str, limit: usize) -> ProviderSuggestions {
    let query = query.trim().to_lowercase();
    let mut matching: Vec<(bool, NotebookView)> = notebooks
        .into_iter()
        .filter_map(|notebook| {
            let name = notebook.name.to_lowercase();
            let starts = name.starts_with(&query);
            (starts || name.contains(&query)).then_some((starts, notebook))
        })
        .collect();
    matching.sort_by(|(left_starts, left), (right_starts, right)| {
        right_starts
            .cmp(left_starts)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
    });
    let incomplete = matching.len() > limit;
    let entries = matching
        .into_iter()
        .take(limit)
        .map(|(_starts, notebook)| MentionEntry {
            token: format!("@{}", notebook.name),
            key: notebook.id,
            label: notebook.name,
            variant: ATTACHED_VARIANT.to_owned(),
            warning: None,
            expands: false,
        })
        .collect();
    ProviderSuggestions::new(entries, incomplete)
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::contracts::{ATTACHED_VARIANT, UtcMillis};

    use super::suggestions;
    use crate::commands::NotebookView;

    fn notebook(id: &str, name: &str, updated_at: i64) -> NotebookView {
        NotebookView {
            id: id.to_owned(),
            name: name.to_owned(),
            sources: Vec::new(),
            created_at: UtcMillis::from_millis(0),
            updated_at: UtcMillis::from_millis(updated_at),
        }
    }

    #[test]
    fn notebooks_that_start_with_the_query_come_first_then_the_most_recently_changed() {
        let notebooks = vec![
            notebook("n1", "Old physics", 1),
            notebook("n2", "Physics", 2),
            notebook("n3", "Applied physics", 3),
            notebook("n4", "Baking", 4),
        ];

        let found = suggestions(notebooks.clone(), " PHYS", 10);
        let few = suggestions(notebooks, "", 2);

        let keys: Vec<&str> = found
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect();
        assert_eq!(keys, ["n2", "n3", "n1"]);
        assert_eq!(found.entries[0].token, "@Physics");
        assert_eq!(found.entries[0].label, "Physics");
        assert_eq!(found.entries[0].variant, ATTACHED_VARIANT);
        assert!(!found.incomplete);
        let recent: Vec<&str> = few.entries.iter().map(|entry| entry.key.as_str()).collect();
        assert_eq!(recent, ["n4", "n3"]);
        assert!(few.incomplete);
    }
}
