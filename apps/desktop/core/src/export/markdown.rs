//! Markdown built from complete records. Generated labels come from the same
//! catalog as the UI; content never chooses filesystem paths.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use mannie_types::clock::UtcMillis;
use mannie_types::export::{ConversationRecord, ExportError, ExportWarning, RecordPart};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The catalog is embedded at build time, so the core needs neither webview
/// text nor filesystem access to translate the exported document's labels.
pub(super) struct Labels(BTreeMap<String, String>);

impl Labels {
    pub(super) fn load() -> Result<Self, ExportError> {
        let catalog = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../messages/en.json"));
        let catalog: BTreeMap<String, serde_json::Value> =
            serde_json::from_str(catalog).map_err(|_error| ExportError::InvalidRecord)?;
        let labels = catalog
            .into_iter()
            .filter_map(|(key, value)| {
                if !key.starts_with("export_") {
                    return None;
                }
                value.as_str().map(|text| (key, text.to_owned()))
            })
            .collect();
        Ok(Self(labels))
    }

    pub(super) fn get(&self, key: &str) -> Result<&str, ExportError> {
        self.0
            .get(key)
            .map(String::as_str)
            .ok_or(ExportError::InvalidRecord)
    }
}

/// Escapes data in generated headings, labels and link descriptions.
fn escape(text: &str) -> String {
    let mut result = String::new();
    for character in text.chars() {
        match character {
            '\n' | '\r' => result.push(' '),
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '!' | '|' => {
                result.push('\\');
                result.push(character);
            }
            character if character.is_control() => {}
            character => result.push(character),
        }
    }
    result
}

/// A fence longer than every backtick run preserves arbitrary saved text.
fn fenced(output: &mut String, language: &str, text: &str) {
    let longest = text
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest.saturating_add(1).max(3));
    let _written = writeln!(output, "{fence}{language}\n{text}\n{fence}\n");
}

fn timestamp(value: UtcMillis) -> Result<String, ExportError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value.as_millis()) * 1_000_000)
        .map_err(|_error| ExportError::InvalidRecord)?
        .format(&Rfc3339)
        .map_err(|_error| ExportError::InvalidRecord)
}

fn field(output: &mut String, labels: &Labels, key: &str, value: &str) -> Result<(), ExportError> {
    let _written = writeln!(output, "- {}: {}", labels.get(key)?, escape(value));
    Ok(())
}

pub(super) fn warning_lines(
    labels: &Labels,
    warnings: &[ExportWarning],
) -> Result<String, ExportError> {
    let mut output = String::new();
    if !warnings.is_empty() {
        let _written = writeln!(
            output,
            "\n## {}\n",
            labels.get("export_document_unavailable")?
        );
        for warning in warnings {
            let code = warning
                .reason
                .code()
                .replacen("export.", "export_error_", 1);
            let _written = writeln!(
                output,
                "- {} / {}: {}",
                warning.conversation_id,
                escape(&warning.reference),
                labels.get(&code)?
            );
        }
    }
    Ok(output)
}

pub(super) fn document(
    record: &ConversationRecord,
    labels: &Labels,
    root: bool,
) -> Result<String, ExportError> {
    let mut output = metadata(record, labels)?;
    for turn in &record.turns {
        render_turn(&mut output, record, turn, labels, root)?;
    }
    if !record.checkpoints.is_empty() {
        let _written = writeln!(
            output,
            "\n## {}\n",
            labels.get("export_document_checkpoints")?
        );
        for checkpoint in &record.checkpoints {
            let _written = writeln!(
                output,
                "### {} {}\n",
                labels.get("export_document_checkpoint")?,
                checkpoint.view.id
            );
            field(
                &mut output,
                labels,
                "export_document_created",
                &timestamp(checkpoint.view.created_at)?,
            )?;
            fenced(&mut output, "text", &checkpoint.summary);
        }
    }
    if !record.children.is_empty() {
        let _written = writeln!(output, "\n## {}\n", labels.get("export_document_children")?);
        for child in &record.children {
            let folder = if root { "tasks/" } else { "" };
            let _written = writeln!(
                output,
                "- [{}]({folder}{}.md)",
                escape(&child.title),
                child.id
            );
        }
    }
    Ok(output)
}

fn metadata(record: &ConversationRecord, labels: &Labels) -> Result<String, ExportError> {
    let mut output = format!(
        "# {}\n\n## {}\n\n",
        escape(&record.conversation.title),
        labels.get("export_document_metadata")?
    );
    field(
        &mut output,
        labels,
        "export_document_id",
        record.conversation.id.as_str(),
    )?;
    field(
        &mut output,
        labels,
        "export_document_created",
        &timestamp(record.conversation.created_at)?,
    )?;
    field(
        &mut output,
        labels,
        "export_document_updated",
        &timestamp(record.conversation.updated_at)?,
    )?;
    field(
        &mut output,
        labels,
        "export_document_captured",
        &timestamp(record.captured_at)?,
    )?;
    field(
        &mut output,
        labels,
        "export_document_sequence",
        &record.through_seq.to_string(),
    )?;
    if let Some(workspace) = &record.conversation.workspace {
        field(&mut output, labels, "export_document_workspace", workspace)?;
    }
    if let Some(task) = &record.conversation.task {
        field(
            &mut output,
            labels,
            "export_document_parent",
            task.parent_id.as_str(),
        )?;
    }
    if record.conversation.has_active_run {
        let _written = writeln!(output, "\n{}\n", labels.get("export_document_active")?);
    }
    let _written = writeln!(output, "\n{}\n", labels.get("export_document_record")?);
    fenced(
        &mut output,
        "json",
        &serde_json::to_string_pretty(&record.conversation)
            .map_err(|_error| ExportError::InvalidRecord)?,
    );
    Ok(output)
}

fn render_turn(
    output: &mut String,
    record: &ConversationRecord,
    turn: &mannie_types::export::RecordTurn,
    labels: &Labels,
    root: bool,
) -> Result<(), ExportError> {
    let _written = writeln!(
        output,
        "\n## {} {}\n",
        labels.get("export_document_turn")?,
        turn.view.position + 1
    );
    field(
        output,
        labels,
        "export_document_created",
        &timestamp(turn.view.created_at)?,
    )?;
    for run in turn.retained_runs.iter().chain(turn.view.run.iter()) {
        field(output, labels, "export_document_state", run.state.as_str())?;
        fenced(
            output,
            "json",
            &serde_json::to_string_pretty(run).map_err(|_error| ExportError::InvalidRecord)?,
        );
    }
    for part in &turn.parts {
        render_part(output, record, part, labels, root)?;
    }
    Ok(())
}

fn render_part(
    output: &mut String,
    record: &ConversationRecord,
    part: &RecordPart,
    labels: &Labels,
    root: bool,
) -> Result<(), ExportError> {
    let key = match part.kind.as_str() {
        "user_text" => "user",
        "text" => "text",
        "reasoning" => "reasoning",
        "tool_call" => "tool_call",
        "task" => "task",
        "mention" => "mention",
        "image" => "image",
        "error" => "error",
        _ => "unknown",
    };
    let _written = writeln!(
        output,
        "\n### {}\n",
        labels.get(&format!("export_document_{key}"))?
    );
    field(output, labels, "export_document_part", part.id.as_str())?;
    field(output, labels, "export_document_kind", &part.kind)?;
    if let Some(run) = &part.run_id {
        field(output, labels, "export_document_run", run.as_str())?;
    }
    field(output, labels, "export_document_state", &part.state)?;
    field(
        output,
        labels,
        "export_document_created",
        &timestamp(part.created_at)?,
    )?;
    let _written = writeln!(output);
    if matches!(part.kind.as_str(), "text" | "reasoning") {
        let _written = writeln!(output, "{}\n", part.text);
    } else if !part.text.is_empty() {
        let language = if matches!(part.kind.as_str(), "tool_call" | "task") {
            "json"
        } else {
            "text"
        };
        fenced(output, language, &part.text);
    }
    let document: serde_json::Value =
        serde_json::from_str(&part.document).map_err(|_error| ExportError::InvalidRecord)?;
    if document.as_object().is_none_or(|object| !object.is_empty()) {
        let _written = writeln!(output, "{}\n", labels.get("export_document_record")?);
        fenced(
            output,
            "json",
            &serde_json::to_string_pretty(&document)
                .map_err(|_error| ExportError::InvalidRecord)?,
        );
    }
    let prefix = if root { "" } else { "../" };
    if part.kind == "task"
        && let Some(id) = document
            .pointer("/task/task/conversationId")
            .and_then(serde_json::Value::as_str)
        && record.children.iter().any(|child| child.id.as_str() == id)
    {
        let folder = if root { "tasks/" } else { "" };
        let _written = writeln!(
            output,
            "[{}]({folder}{id}.md)\n",
            labels.get("export_document_task")?
        );
    }
    if let Some(image) = record.images.iter().find(|image| image.part_id == part.id) {
        let name = document
            .pointer("/image/view/name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(labels.get("export_document_image")?);
        let _written = writeln!(
            output,
            "![{}]({prefix}images/{}.{})\n",
            escape(name),
            image.part_id,
            image.format.as_str()
        );
    }
    for archive in record
        .archives
        .iter()
        .filter(|archive| archive.part_id == part.id)
    {
        let _written = writeln!(
            output,
            "[{}: {}]({prefix}outputs/{}.txt)\n",
            labels.get("export_document_output")?,
            escape(&archive.name),
            archive.id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_labels_escape_markup_and_fences_preserve_saved_text() {
        assert_eq!(escape("[title]\n# !<é>"), "\\[title\\] \\# \\!\\<é\\>");
        let mut output = String::new();
        fenced(&mut output, "text", "before\n````\nafter");
        assert!(output.starts_with("`````text\n"));
        assert!(output.ends_with("\n`````\n\n"));
    }

    #[test]
    fn every_warning_has_catalog_text_and_timestamps_are_utc() {
        let labels = Labels::load().unwrap();
        let id = mannie_types::ids::ConversationId::parse("0199e0f2-0000-7000-8000-000000000001")
            .unwrap();
        let warnings: Vec<_> = [
            ExportError::ConversationNotFound,
            ExportError::StorageFailed,
            ExportError::StoreUnavailable,
            ExportError::DialogFailed,
            ExportError::WriteFailed,
            ExportError::ExportActive,
            ExportError::Unsupported,
            ExportError::HostUnavailable,
            ExportError::PluginOff,
            ExportError::AttachmentMissing,
            ExportError::InvalidRecord,
        ]
        .into_iter()
        .map(|reason| ExportWarning {
            conversation_id: id.clone(),
            reference: "[missing]".to_owned(),
            reason,
        })
        .collect();
        let output = warning_lines(&labels, &warnings).unwrap();
        assert!(output.contains("Unavailable content"));
        assert!(output.contains("\\[missing\\]"));
        assert_eq!(timestamp(UtcMillis::EPOCH).unwrap(), "1970-01-01T00:00:00Z");
    }
}

#[cfg(test)]
mod record_tests {
    use super::*;
    use mannie_types::export::{RECORD_VERSION, RecordImage, RecordPart, RecordTurn};
    use mannie_types::ids::{ConversationId, PartId, TurnId};
    use mannie_types::images::ImageFormat;
    use mannie_types::transcript::{ConversationSummary, TitleSource, TurnView};

    pub(super) fn record() -> ConversationRecord {
        let conversation = ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();
        let part = PartId::parse("0199e0f2-0000-7000-8000-000000000002").unwrap();
        let turn = TurnId::parse("0199e0f2-0000-7000-8000-000000000003").unwrap();
        ConversationRecord {
            version: RECORD_VERSION,
            conversation: ConversationSummary {
                id: conversation,
                title: "[A title] é".to_owned(),
                title_source: TitleSource::Owner,
                profile_id: None,
                has_active_run: true,
                awaiting_approval: false,
                full_access: false,
                project_id: None,
                workspace: None,
                task: None,
                created_at: UtcMillis::EPOCH,
                updated_at: UtcMillis::EPOCH,
            },
            captured_at: UtcMillis::EPOCH,
            through_seq: 4,
            turns: vec![RecordTurn {
                view: TurnView {
                    id: turn,
                    position: 0,
                    run: None,
                    parts: Vec::new(),
                    created_at: UtcMillis::EPOCH,
                },
                retained_runs: Vec::new(),
                parts: vec![RecordPart {
                    id: part.clone(),
                    kind: "image".to_owned(),
                    state: "complete".to_owned(),
                    run_id: None,
                    position: 0,
                    text: String::new(),
                    document: "{}".to_owned(),
                    created_at: UtcMillis::EPOCH,
                }],
            }],
            checkpoints: Vec::new(),
            images: vec![RecordImage {
                part_id: part,
                format: ImageFormat::Png,
                data: b"saved-image".to_vec(),
            }],
            archives: Vec::new(),
            children: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn markdown_links_use_ids_and_relocate_from_child_documents() {
        let record = record();
        let labels = Labels::load().unwrap();
        let root = document(&record, &labels, true).unwrap();
        let child = document(&record, &labels, false).unwrap();
        assert!(root.contains("# \\[A title\\] é"));
        assert!(root.contains("](images/0199e0f2-0000-7000-8000-000000000002.png)"));
        assert!(child.contains("](../images/0199e0f2-0000-7000-8000-000000000002.png)"));
        assert!(root.contains("still running"));
        assert!(root.contains("\"profileId\": null"));
        assert!(root.contains("\"hasActiveRun\": true"));
    }

    #[test]
    fn renderer_and_writer_preserve_stored_asset_bytes() {
        let record = record();
        let parent = tempfile::tempdir().unwrap();
        let mut folder =
            crate::export::folder::Folder::create(parent.path().join("export"), "incomplete")
                .unwrap();
        folder.assets(&record).unwrap();
        let image = folder
            .path()
            .join("images/0199e0f2-0000-7000-8000-000000000002.png");
        assert_eq!(std::fs::read(image).unwrap(), b"saved-image");
    }
}
