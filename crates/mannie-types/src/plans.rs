//! Plans (ADR-0060; docs/features/plans.md): what an agent saved for the owner
//! to read, edit and run later, and the errors of the feature's commands.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::{ConversationId, ProjectId};
use crate::labels::clean_label;

/// The most characters of a plan's title.
pub const MAX_PLAN_TITLE_CHARS: usize = 80;

/// The most bytes of a plan's text.
pub const MAX_PLAN_BODY_BYTES: usize = 32 * 1024;

/// A plan in a list, without its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    /// Its number, which never returns after a delete.
    pub number: u32,
    /// Its title.
    pub title: String,
    /// The conversation that saved it, while that exists.
    pub conversation_id: Option<ConversationId>,
    /// The project of that conversation, while that exists.
    pub project_id: Option<ProjectId>,
    /// When it last changed.
    pub updated_at: UtcMillis,
}

/// A plan with its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    /// What a list shows of it.
    pub summary: PlanSummary,
    /// Its text, in Markdown.
    pub body: String,
}

/// The argument of `get_plan` and `delete_plan`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlanRef {
    /// The plan.
    pub number: u32,
}

/// The argument of `update_plan`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePlan {
    /// The plan.
    pub number: u32,
    /// Its title from now on.
    pub title: String,
    /// Its text from now on.
    pub body: String,
}

/// A title and a text that passed their limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedPlan {
    /// The title, cleaned as a label is.
    pub title: String,
    /// The text, as it was written.
    pub body: String,
}

/// What is wrong with a plan's title or text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PlanField {
    /// The title is blank or over its limit.
    Title,
    /// The text is blank or over its limit.
    Body,
}

/// Checks a plan's title and text. Text over a limit is refused and not cut, so
/// a plan never changes by being cut (ADR-0060, decision 4).
pub fn check_plan(title: &str, body: &str) -> Result<CheckedPlan, PlanField> {
    if title.chars().count() > MAX_PLAN_TITLE_CHARS {
        return Err(PlanField::Title);
    }
    let title = clean_label(title, MAX_PLAN_TITLE_CHARS).ok_or(PlanField::Title)?;
    if body.trim().is_empty() || body.len() > MAX_PLAN_BODY_BYTES {
        return Err(PlanField::Body);
    }
    Ok(CheckedPlan {
        title,
        body: body.to_owned(),
    })
}

/// Why a command of plans failed. The UI shows the message for the code from
/// its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PlanError {
    /// The plan does not exist.
    #[error("the plan does not exist")]
    NotFound,
    /// The title or the text is not valid.
    #[error("the plan's title or text is not valid")]
    FieldNotValid {
        /// The field.
        field: PlanField,
    },
    /// The store failed.
    #[error("the plans could not be read or saved")]
    StorageFailed,
    /// The store is not available.
    #[error("the store is not available")]
    StoreUnavailable,
}

impl PlanError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "plans.not_found",
            Self::FieldNotValid { .. } => "plans.field_not_valid",
            Self::StorageFailed => "plans.storage_failed",
            Self::StoreUnavailable => "plans.store_unavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_within_its_limits_comes_back_with_its_title_cleaned() {
        let checked = check_plan("  Add\ndark  mode ", "# Steps\n").unwrap();

        assert_eq!(checked.title, "Add dark mode");
        assert_eq!(checked.body, "# Steps\n");
    }

    #[test]
    fn text_over_a_limit_or_blank_is_refused_and_not_cut() {
        let long_title = "t".repeat(MAX_PLAN_TITLE_CHARS + 1);
        assert_eq!(check_plan(&long_title, "Body"), Err(PlanField::Title));
        assert_eq!(check_plan(" \n", "Body"), Err(PlanField::Title));
        let long_body = "b".repeat(MAX_PLAN_BODY_BYTES + 1);
        assert_eq!(check_plan("Title", &long_body), Err(PlanField::Body));
        assert_eq!(check_plan("Title", "  "), Err(PlanField::Body));
        let at_the_limit = "b".repeat(MAX_PLAN_BODY_BYTES);
        check_plan("Title", &at_the_limit).unwrap();
    }
}
