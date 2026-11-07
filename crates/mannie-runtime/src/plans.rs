//! Plans (ADR-0060; docs/features/plans.md): the tool `save_plan` as a model
//! receives it, the checking of its calls, the provider of plans' mentions, and
//! the commands of plans.
//!
//! The tool changes nothing on the owner's machine beyond Mannie's own database,
//! so it has no accesses and is no operation, and an agent that only reads may
//! have it. Its part records it, in the same transaction as the plan.
//!
//! A plan reaches a later reply only as a mention, whose token holds the plan's
//! number and never its title. The policy reads the owner's words from their
//! own text, which holds the token, so a model-written title cannot name a
//! project and allow a move into it (T-84).

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugin_sdk::points::mentions::{MentionContent, ProviderSuggestions, RenderedMention};
use mannie_provider::ToolDefinition;
use mannie_store::{RuntimeStoreError, Store, StoredPlan};
use mannie_tools::{Done, with_activity};
use mannie_types::mentions::{MentionEntry, MentionSummary};
use mannie_types::plans::{
    CheckedPlan, MAX_PLAN_BODY_BYTES, MAX_PLAN_TITLE_CHARS, PlanError, PlanField, PlanSummary,
    PlanView, UpdatePlan, check_plan,
};
use mannie_types::tools::{ToolCallDocument, ToolOutcome};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mentions::{MentionContext, MentionProvider};
use crate::{Runtime, texts};

/// The name of the tool that saves a plan.
pub const SAVE_PLAN: &str = "save_plan";

/// The ID of the provider of plans' mentions.
pub const PLANS: &str = "plans";

/// The variant of a plan's mention, which chooses its icon.
const PLAN_VARIANT: &str = "plan";

/// The code of a mention that holds the whole plan.
const SUMMARY_WHOLE: &str = "whole";

/// The code of a mention that holds the start of the plan.
const SUMMARY_CUT: &str = "cut";

/// The code of a mention of a plan that does not exist.
const PLAN_NOT_FOUND: &str = "plan_not_found";

/// The code of a mention of a plan that could not be read.
const PLAN_UNREADABLE: &str = "plan_unreadable";

/// The arguments of `save_plan`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    title: String,
    body: String,
}

/// The definition of `save_plan`, as a model receives it.
pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: SAVE_PLAN.to_owned(),
        description: texts::SAVE_PLAN.to_owned(),
        parameters: with_activity(&parameters()),
        strict: true,
    }
}

fn parameters() -> Value {
    json!({
        "additionalProperties": false,
        "properties": {
            "body": {
                "description": texts::PLAN_BODY_ARGUMENT,
                "type": "string",
            },
            "title": {
                "description": texts::PLAN_TITLE_ARGUMENT,
                "type": "string",
            },
        },
        "required": ["body", "title"],
        "type": "object",
    })
}

/// Checks a call's arguments, without the activity, which the runtime took out.
/// The error says what is wrong, for the model.
pub(crate) fn parse_call(arguments: &str) -> Result<CheckedPlan, String> {
    let input: Arguments =
        serde_json::from_str(arguments.trim()).map_err(|error| error.to_string())?;
    check_plan(&input.title, &input.body).map_err(|field| match field {
        PlanField::Title => {
            format!("the title is blank or longer than {MAX_PLAN_TITLE_CHARS} characters")
        }
        PlanField::Body => {
            format!("the plan is blank or longer than {MAX_PLAN_BODY_BYTES} bytes")
        }
    })
}

fn done(outcome: ToolOutcome, output: String) -> Done {
    Done {
        outcome,
        output,
        archives: Vec::new(),
        images: Vec::new(),
        cited: Vec::new(),
    }
}

/// The result of a call that saved the plan.
pub(crate) fn saved(title: String) -> Done {
    done(
        ToolOutcome::PlanSaved { title },
        texts::PLAN_SAVED.to_owned(),
    )
}

/// The result of a call that saved nothing, because the owner edited the plan,
/// which holds the plan as it stands, so the model works from the owner's text
/// (ADR-0060, decision 5).
pub(crate) fn kept(plan: &StoredPlan) -> Done {
    done(
        ToolOutcome::PlanKept,
        texts::plan_kept(&plan.title, &plan.body),
    )
}

/// Whether a call saved a plan or read the owner's edit. Its result stays in the
/// model's context, because the note that replaces a result asks the model to
/// run the tool again, which would save the plan again.
pub(crate) fn holds_plan(call: &ToolCallDocument) -> bool {
    call.result.as_ref().is_some_and(|result| {
        matches!(
            result.outcome,
            ToolOutcome::PlanSaved { .. } | ToolOutcome::PlanKept
        )
    })
}

/// Whether a call saved a plan, whose text the model edits from.
pub(crate) fn holds_saved_plan(call: &ToolCallDocument) -> bool {
    call.result
        .as_ref()
        .is_some_and(|result| matches!(result.outcome, ToolOutcome::PlanSaved { .. }))
}

/// What a plan's mention gives the model.
struct PlanContent {
    plan: StoredPlan,
}

impl MentionContent for PlanContent {
    fn render(&self, limit: usize) -> RenderedMention {
        let heading = texts::plan_mention_heading(self.plan.number, &self.plan.title);
        let whole = format!("{heading}\n\n{}", self.plan.body);
        let total = u32::try_from(self.plan.body.len()).unwrap_or(u32::MAX);
        let (text, summary) = if whole.len() <= limit {
            (
                whole,
                MentionSummary {
                    code: SUMMARY_WHOLE.to_owned(),
                    values: [("total".to_owned(), total)].into(),
                },
            )
        } else {
            cut_mention(&heading, &self.plan.body, limit, total)
        };
        RenderedMention::new(
            text,
            self.plan.title.clone(),
            PLAN_VARIANT.to_owned(),
            summary,
        )
    }
}

/// The start of a plan that does not fit, with a note that says what is missing,
/// because the core's own cut would add none (ADR-0060, decision 6).
fn cut_mention(heading: &str, body: &str, limit: usize, total: u32) -> (String, MentionSummary) {
    // The note's numbers are short, so room for these digits always holds them.
    let note_room = texts::plan_cut_note(u32::MAX, u32::MAX).len();
    let room = limit.saturating_sub(heading.len() + note_room + 2);
    let shown_text = &body[..body.floor_char_boundary(room)];
    let shown = u32::try_from(shown_text.len()).unwrap_or(u32::MAX);
    let text = format!(
        "{heading}\n\n{shown_text}{}",
        texts::plan_cut_note(shown, total)
    );
    (
        text,
        MentionSummary {
            code: SUMMARY_CUT.to_owned(),
            values: [("shown".to_owned(), shown), ("total".to_owned(), total)].into(),
        },
    )
}

/// The core's provider of plans' mentions (ADR-0060, decision 6). It is offered
/// everywhere, because a plan can be run in any conversation.
#[derive(Debug, Clone)]
pub struct PlanMentions {
    store: Store,
}

impl PlanMentions {
    /// The provider, which reads plans from `store`.
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

/// The token of a plan's mention. It holds the number and never the title: a
/// token enters the owner's own text, which the policy reads for a project's
/// name, and a title is model output (T-84).
pub fn plan_token(number: u32) -> String {
    format!("@plan-{number}")
}

impl MentionProvider for PlanMentions {
    fn id(&self) -> &str {
        PLANS
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
        async move {
            let plans = self.store.plans().await.map_err(|error| {
                tracing::warn!(%error, error.code = error.code(), "could not read the plans for a mention");
                PLAN_UNREADABLE.to_owned()
            })?;
            let query = query.to_lowercase();
            let matching: Vec<StoredPlan> = plans
                .into_iter()
                .filter(|plan| {
                    plan.title.to_lowercase().contains(&query)
                        || plan_token(plan.number).contains(&query)
                })
                .collect();
            let incomplete = matching.len() > limit;
            let entries = matching
                .into_iter()
                .take(limit)
                .map(|plan| MentionEntry {
                    key: plan.number.to_string(),
                    token: plan_token(plan.number),
                    label: plan.title,
                    variant: PLAN_VARIANT.to_owned(),
                    warning: None,
                    expands: false,
                })
                .collect();
            Ok(ProviderSuggestions::new(entries, incomplete))
        }
        .boxed()
    }

    fn resolve<'a>(
        &'a self,
        _context: &'a MentionContext,
        key: &'a str,
        _max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        async move {
            // The key comes from the webview, so it is checked again.
            let number: u32 = key.parse().map_err(|_not_a_number| PLAN_NOT_FOUND.to_owned())?;
            let plan = self
                .store
                .plan(number)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, error.code = error.code(), "could not read a plan for a mention");
                    PLAN_UNREADABLE.to_owned()
                })?
                .ok_or_else(|| PLAN_NOT_FOUND.to_owned())?;
            let content: Box<dyn MentionContent> = Box::new(PlanContent { plan });
            Ok(content)
        }
        .boxed()
    }
}

fn store_failure(error: &RuntimeStoreError) -> PlanError {
    tracing::error!(%error, error.code = error.code(), "the plans could not be read or saved");
    PlanError::StorageFailed
}

fn summary_of(plan: &StoredPlan) -> PlanSummary {
    PlanSummary {
        number: plan.number,
        title: plan.title.clone(),
        conversation_id: plan.conversation_id.clone(),
        project_id: plan.project_id.clone(),
        updated_at: plan.updated_at,
    }
}

impl Runtime {
    /// The plans, the one that changed last first.
    pub async fn list_plans(&self) -> Result<Vec<PlanSummary>, PlanError> {
        let plans = self
            .inner
            .store
            .plans()
            .await
            .map_err(|error| store_failure(&error))?;
        Ok(plans.iter().map(summary_of).collect())
    }

    /// One plan, with its text.
    pub async fn get_plan(&self, number: u32) -> Result<PlanView, PlanError> {
        let plan = self
            .inner
            .store
            .plan(number)
            .await
            .map_err(|error| store_failure(&error))?
            .ok_or(PlanError::NotFound)?;
        Ok(PlanView {
            summary: summary_of(&plan),
            body: plan.body,
        })
    }

    /// The owner's edit of a plan, which the model reads before it saves the
    /// plan again.
    pub async fn update_plan(&self, input: UpdatePlan) -> Result<PlanView, PlanError> {
        let checked = check_plan(&input.title, &input.body)
            .map_err(|field| PlanError::FieldNotValid { field })?;
        let found = self
            .inner
            .store
            .edit_plan(input.number, &checked.title, &checked.body)
            .await
            .map_err(|error| store_failure(&error))?;
        if !found {
            return Err(PlanError::NotFound);
        }
        self.get_plan(input.number).await
    }

    /// Deletes a plan.
    pub async fn delete_plan(&self, number: u32) -> Result<(), PlanError> {
        let found = self
            .inner
            .store
            .delete_plan(number)
            .await
            .map_err(|error| store_failure(&error))?;
        if found {
            Ok(())
        } else {
            Err(PlanError::NotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use mannie_types::clock::UtcMillis;

    use super::*;

    fn plan(body: &str) -> PlanContent {
        PlanContent {
            plan: StoredPlan {
                number: 7,
                conversation_id: None,
                project_id: None,
                title: "Add dark mode".to_owned(),
                body: body.to_owned(),
                owner_edited: false,
                created_at: UtcMillis::EPOCH,
                updated_at: UtcMillis::EPOCH,
            },
        }
    }

    #[test]
    fn a_plan_that_fits_comes_whole_under_its_heading() {
        let rendered = plan("Step one.").render(10_000);

        assert_eq!(
            rendered.text,
            format!(
                "{}\n\nStep one.",
                texts::plan_mention_heading(7, "Add dark mode")
            )
        );
        assert_eq!(rendered.label, "Add dark mode");
        assert_eq!(rendered.summary.code, SUMMARY_WHOLE);
    }

    #[test]
    fn a_plan_that_does_not_fit_says_what_is_missing_within_its_share() {
        let limit = 1_000;
        let rendered = plan(&"é".repeat(2_000)).render(limit);

        assert!(rendered.text.len() <= limit, "{}", rendered.text.len());
        assert_eq!(rendered.summary.code, SUMMARY_CUT);
        let shown = rendered.summary.values["shown"];
        assert_eq!(rendered.summary.values["total"], 4_000);
        assert!(
            rendered.text.ends_with(&texts::plan_cut_note(shown, 4_000)),
            "{}",
            rendered.text
        );
    }

    #[test]
    fn a_share_too_small_for_the_note_still_never_panics() {
        let rendered = plan("Step one.").render(4);

        assert_eq!(rendered.summary.code, SUMMARY_CUT);
        assert_eq!(rendered.summary.values["shown"], 0);
    }

    #[test]
    fn a_token_holds_the_number_and_never_the_title() {
        assert_eq!(plan_token(12), "@plan-12");
    }

    #[test]
    fn the_arguments_are_checked_for_the_model() {
        let valid = json!({ "title": "A plan", "body": "Do it." }).to_string();
        assert_eq!(parse_call(&valid).unwrap().title, "A plan");

        let blank = json!({ "title": "A plan", "body": "  " }).to_string();
        assert!(parse_call(&blank).unwrap_err().contains("blank"));

        let long = json!({ "title": "t".repeat(81), "body": "Do it." }).to_string();
        assert!(parse_call(&long).unwrap_err().contains("title"));

        let other = json!({ "title": "A plan", "body": "Do it.", "extra": 1 }).to_string();
        assert!(parse_call(&other).unwrap_err().contains("extra"));
        parse_call("not json").unwrap_err();
    }

    #[test]
    fn the_schema_is_strict_at_every_level() {
        let schema = parameters();
        assert_eq!(schema["additionalProperties"], false);
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect();
        assert_eq!(required, ["body", "title"]);
        assert_eq!(schema["properties"].as_object().unwrap().len(), 2);
    }
}
