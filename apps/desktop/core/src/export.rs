//! Owner-initiated export. The native picker chooses the destination; Rust
//! collects saved records, journals execution and writes the Markdown folder.

mod folder;
mod markdown;

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use mannie_runtime::Runtime;
use mannie_store::Store;
use mannie_types::export::{
    ConversationRecord, ExportError, ExportOutcome, ExportWarning, RECORD_VERSION,
};
use mannie_types::ids::{ConversationId, ExportId};
use mannie_types::transcript::ConversationSummary;
use tauri::AppHandle;
use tokio::sync::Semaphore;

use folder::Folder;
use markdown::Labels;

/// The export gate belongs to the desktop instance, not to global state.
#[derive(Debug)]
pub(crate) struct Exports {
    gate: Arc<Semaphore>,
}

impl Default for Exports {
    fn default() -> Self {
        Self {
            gate: Arc::new(Semaphore::new(1)),
        }
    }
}

impl Exports {
    pub(crate) async fn run(
        &self,
        app: &AppHandle,
        runtime: &Runtime,
        store: &Store,
        conversation: &ConversationId,
    ) -> Result<ExportOutcome, ExportError> {
        let _permit = Arc::clone(&self.gate)
            .try_acquire_owned()
            .map_err(|_busy| ExportError::ExportActive)?;
        let Some(parent) = crate::folder_dialog::choose_folder(app)
            .await
            .map_err(|_dialog| ExportError::DialogFailed)?
        else {
            return Ok(ExportOutcome::Canceled);
        };
        let root = runtime.export_record(conversation).await?;
        let operation = runtime.new_export_id();
        let parent = tokio::task::spawn_blocking(move || std::fs::canonicalize(parent))
            .await
            .map_err(|_join| ExportError::WriteFailed)?
            .map_err(|_io| ExportError::WriteFailed)?;
        let destination = parent.join(format!("conversation-{conversation}-{operation}"));
        let path = destination
            .to_str()
            .ok_or(ExportError::WriteFailed)?
            .to_owned();
        store
            .begin_export(&operation, conversation, &path)
            .await
            .map_err(|_store| ExportError::StorageFailed)?;
        let result = write_export(runtime, store, &operation, destination, root).await;
        if result.is_err()
            && let Err(error) = store.finish_export(&operation, false).await
        {
            tracing::error!(
                error.code = error.code(),
                "the failed export could not be journaled"
            );
        }
        result
    }
}

async fn write_export(
    runtime: &Runtime,
    store: &Store,
    operation: &ExportId,
    destination: PathBuf,
    root: ConversationRecord,
) -> Result<ExportOutcome, ExportError> {
    if root.version != RECORD_VERSION {
        return Err(ExportError::InvalidRecord);
    }
    let captured_at = root.captured_at;
    let root_id = root.conversation.id.clone();
    let mut seen = HashSet::from([root_id]);
    let mut queue: VecDeque<_> = root.children.clone().into();
    let mut warnings = root.warnings.clone();
    let (mut folder, main) = tokio::task::spawn_blocking(move || {
        let labels = Labels::load()?;
        let mut folder = Folder::create(destination, labels.get("export_document_incomplete")?)?;
        folder.assets(&root)?;
        let main = markdown::document(&root, &labels, true)?;
        Ok::<_, ExportError>((folder, main))
    })
    .await
    .map_err(|_join| ExportError::WriteFailed)??;
    while let Some(summary) = queue.pop_front() {
        if !seen.insert(summary.id.clone()) {
            continue;
        }
        let mut record = match runtime.export_record(&summary.id).await {
            Ok(record) => record,
            Err(reason) => {
                let warning = ExportWarning {
                    conversation_id: summary.id.clone(),
                    reference: summary.id.to_string(),
                    reason,
                };
                unavailable_record(summary, captured_at, warning)
            }
        };
        for child in &record.children {
            if child
                .task
                .as_ref()
                .is_some_and(|task| task.parent_id == record.conversation.id)
            {
                queue.push_back(child.clone());
            } else {
                record.warnings.push(ExportWarning {
                    conversation_id: record.conversation.id.clone(),
                    reference: child.id.to_string(),
                    reason: ExportError::InvalidRecord,
                });
            }
        }
        warnings.extend(record.warnings.clone());
        folder = tokio::task::spawn_blocking(move || {
            let labels = Labels::load()?;
            folder.assets(&record)?;
            let mut text = markdown::document(&record, &labels, false)?;
            text.push_str(&markdown::warning_lines(&labels, &record.warnings)?);
            folder.write(
                &format!("tasks/{}.md", record.conversation.id),
                text.as_bytes(),
            )?;
            Ok::<_, ExportError>(folder)
        })
        .await
        .map_err(|_join| ExportError::WriteFailed)??;
    }
    let final_warnings = warnings.clone();
    folder = tokio::task::spawn_blocking(move || {
        let labels = Labels::load()?;
        let mut main = main;
        main.push_str(&markdown::warning_lines(&labels, &final_warnings)?);
        folder.write("conversation.md", main.as_bytes())?;
        folder.seal()?;
        Ok::<_, ExportError>(folder)
    })
    .await
    .map_err(|_join| ExportError::WriteFailed)??;
    store
        .finish_export(operation, true)
        .await
        .map_err(|_store| ExportError::StorageFailed)?;
    let destination = folder
        .path()
        .to_str()
        .ok_or(ExportError::WriteFailed)?
        .to_owned();
    folder.keep();
    tracing::info!(operation = %operation, omissions = warnings.len(), "exported a conversation");
    Ok(ExportOutcome::Saved {
        destination,
        warnings,
    })
}

fn unavailable_record(
    conversation: ConversationSummary,
    captured_at: mannie_types::clock::UtcMillis,
    warning: ExportWarning,
) -> ConversationRecord {
    ConversationRecord {
        version: RECORD_VERSION,
        conversation,
        captured_at,
        through_seq: 0,
        turns: Vec::new(),
        checkpoints: Vec::new(),
        images: Vec::new(),
        archives: Vec::new(),
        children: Vec::new(),
        warnings: vec![warning],
        background_commands: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
