//! Questions to the owner (ADR-0058; docs/features/questions.md): the questions
//! of a call of `ask_owner`, as the core checked and cleaned them, and the
//! owner's reply to them.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{ConversationId, PartId};

/// The most questions of one call.
pub const MAX_QUESTIONS: usize = 4;

/// The fewest options of one question.
pub const MIN_OPTIONS: usize = 2;

/// The most options of one question.
pub const MAX_OPTIONS: usize = 6;

/// The most characters of a question's text.
pub const MAX_QUESTION_CHARS: usize = 1_000;

/// The most characters of an option's label.
pub const MAX_OPTION_LABEL_CHARS: usize = 100;

/// The most characters of an option's description.
pub const MAX_OPTION_DESCRIPTION_CHARS: usize = 300;

/// The most bytes of the owner's own words in the answer to one question.
pub const MAX_OWN_WORDS_BYTES: usize = 16 * 1024;

/// One option of a question. The model wrote its texts, so they are content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    /// What the option is called, unique within its question.
    pub label: String,
    /// What choosing it means, or `None` when the model wrote nothing.
    pub description: Option<String>,
}

/// One question of a call of `ask_owner`. The model wrote its text, so it is
/// content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    /// The question's text.
    pub text: String,
    /// Whether the owner can choose several options.
    pub multiple: bool,
    /// The options, in the model's order.
    pub options: Vec<QuestionOption>,
}

/// The owner's answer to one question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    /// The positions of the options that the owner chose, from 0.
    pub selected: Vec<u32>,
    /// What the owner wrote instead of choosing, or beside their choices in a
    /// question with several.
    pub own_words: Option<String>,
}

/// The owner's reply to the questions of a call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "reply",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum QuestionReply {
    /// The owner answered every question, in the questions' order.
    Answered {
        /// One answer for each question.
        answers: Vec<QuestionAnswer>,
    },
    /// The owner chose not to answer.
    Declined,
}

/// The argument of `answer_question`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AnswerQuestion {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The part of the call of `ask_owner`.
    pub part_id: PartId,
    /// The owner's reply.
    pub reply: QuestionReply,
}

#[cfg(test)]
mod tests {
    use super::{QuestionAnswer, QuestionReply};

    #[test]
    fn a_reply_serializes_with_its_tag() {
        let answered = QuestionReply::Answered {
            answers: vec![QuestionAnswer {
                selected: vec![1],
                own_words: None,
            }],
        };

        assert_eq!(
            serde_json::to_value(&answered).unwrap(),
            serde_json::json!({
                "reply": "answered",
                "answers": [{ "selected": [1], "ownWords": null }],
            })
        );
        assert_eq!(
            serde_json::to_value(QuestionReply::Declined).unwrap(),
            serde_json::json!({ "reply": "declined" })
        );
    }
}
