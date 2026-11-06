//! Questions to the owner (ADR-0058; docs/features/questions.md): the tool
//! `ask_owner` as a model receives it, the checking of its calls, the reply
//! that a run waits for, and what the model reads of it.
//!
//! The tool acts on its run and not on the machine, so it stays outside the
//! registry, as the task tools do. The run actor drives the wait
//! (`actor/operations.rs`). The run registers a one-shot reply here, as it
//! does for an approval (`approvals.rs`), so its mailbox keeps carrying Stop
//! and Shutdown only, and a run that waits for a reply still stops at once.
//!
//! The schema is written here, strict at every level. A test checks it against
//! the types that parse the arguments, and `tests/questions.rs` compares it
//! with a reviewed snapshot in `tests/fixtures/question_definition.json`.

use mannie_provider::ToolDefinition;
use mannie_tools::{Done, clean_label, with_activity};
use mannie_types::ids::{ConversationId, PartId};
use mannie_types::tools::{
    AnswerQuestion, MAX_OPTION_DESCRIPTION_CHARS, MAX_OPTION_LABEL_CHARS, MAX_OPTIONS,
    MAX_OWN_WORDS_BYTES, MAX_QUESTION_CHARS, MAX_QUESTIONS, MIN_OPTIONS, Question, QuestionAnswer,
    QuestionOption, QuestionReply, ToolCallDocument, ToolOutcome, ToolsError,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::{Runtime, lock, texts};

/// The tool that asks the owner questions.
pub(crate) const ASK_OWNER: &str = "ask_owner";

/// The arguments of `ask_owner`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    questions: Vec<QuestionArgument>,
}

/// One question of the arguments of `ask_owner`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionArgument {
    question: String,
    multiple: bool,
    options: Vec<OptionArgument>,
}

/// One option of the arguments of `ask_owner`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionArgument {
    label: String,
    description: String,
}

/// The definition of `ask_owner`, as a model receives it.
pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: ASK_OWNER.to_owned(),
        description: texts::ask_owner(),
        parameters: with_activity(&parameters()),
        strict: true,
    }
}

fn parameters() -> Value {
    json!({
        "additionalProperties": false,
        "properties": {
            "questions": {
                "description": texts::QUESTIONS_ARGUMENT,
                "items": {
                    "additionalProperties": false,
                    "properties": {
                        "multiple": {
                            "description": texts::QUESTION_MULTIPLE_ARGUMENT,
                            "type": "boolean",
                        },
                        "options": {
                            "description": texts::QUESTION_OPTIONS_ARGUMENT,
                            "items": {
                                "additionalProperties": false,
                                "properties": {
                                    "description": {
                                        "description": texts::OPTION_DESCRIPTION_ARGUMENT,
                                        "type": "string",
                                    },
                                    "label": {
                                        "description": texts::OPTION_LABEL_ARGUMENT,
                                        "type": "string",
                                    },
                                },
                                "required": ["description", "label"],
                                "type": "object",
                            },
                            "type": "array",
                        },
                        "question": {
                            "description": texts::QUESTION_TEXT_ARGUMENT,
                            "type": "string",
                        },
                    },
                    "required": ["multiple", "options", "question"],
                    "type": "object",
                },
                "type": "array",
            },
        },
        "required": ["questions"],
        "type": "object",
    })
}

/// A text that the model wrote for the owner, as the owner reads it, or `None`
/// when nothing is left of it. A text over its limit is refused, so that a
/// question never changes by being cut, and the model can write it shorter.
fn shown_text(text: &str, max_chars: usize, what: &str) -> Result<Option<String>, String> {
    if text.chars().count() > max_chars {
        return Err(format!("{what} is longer than {max_chars} characters"));
    }
    Ok(clean_label(text, max_chars))
}

/// Checks one option.
fn checked_option(input: &OptionArgument) -> Result<QuestionOption, String> {
    let label = shown_text(&input.label, MAX_OPTION_LABEL_CHARS, "an option's label")?
        .ok_or_else(|| "an option's label is empty".to_owned())?;
    let description = shown_text(
        &input.description,
        MAX_OPTION_DESCRIPTION_CHARS,
        "an option's description",
    )?;
    Ok(QuestionOption { label, description })
}

/// Checks one question and its options.
fn checked_question(input: &QuestionArgument) -> Result<Question, String> {
    let text = shown_text(&input.question, MAX_QUESTION_CHARS, "a question")?
        .ok_or_else(|| "a question is empty".to_owned())?;
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&input.options.len()) {
        return Err(format!(
            "a question has {MIN_OPTIONS} to {MAX_OPTIONS} options"
        ));
    }
    let options = input
        .options
        .iter()
        .map(checked_option)
        .collect::<Result<Vec<_>, _>>()?;
    for (position, option) in options.iter().enumerate() {
        let repeated = options[..position]
            .iter()
            .any(|earlier| earlier.label == option.label);
        if repeated {
            return Err(format!(
                "a question has the option {:?} twice",
                option.label
            ));
        }
    }
    Ok(Question {
        text,
        multiple: input.multiple,
        options,
    })
}

/// Checks a call of `ask_owner`, without the activity, which the runtime took
/// out. The error says what is wrong, for the model.
pub(crate) fn parse_call(arguments: &str) -> Result<Vec<Question>, String> {
    let input: Arguments = serde_json::from_str(arguments).map_err(|error| error.to_string())?;
    if input.questions.is_empty() || input.questions.len() > MAX_QUESTIONS {
        return Err(format!("a call asks 1 to {MAX_QUESTIONS} questions"));
    }
    input.questions.iter().map(checked_question).collect()
}

/// Whether an answer fits its question: positions of its options, each once,
/// and own words that are not blank, with exactly one answer to a question of
/// one choice and at least one to a question of several.
fn answer_fits(question: &Question, answer: &QuestionAnswer) -> bool {
    let selected = &answer.selected;
    if selected.len() > question.options.len() {
        return false;
    }
    let known_options = selected.iter().enumerate().all(|(position, option)| {
        usize::try_from(*option).is_ok_and(|option| option < question.options.len())
            && !selected[..position].contains(option)
    });
    let own_words_fit = answer
        .own_words
        .as_deref()
        .is_none_or(|words| !words.trim().is_empty() && words.len() <= MAX_OWN_WORDS_BYTES);
    let given = selected.len() + usize::from(answer.own_words.is_some());
    let enough = if question.multiple {
        given >= 1
    } else {
        given == 1
    };
    known_options && own_words_fit && enough
}

/// Whether a reply fits the questions that wait for it. The webview is
/// untrusted (ADR-0004), so the core checks what it sent, and the result is
/// written from the core's own copy of the questions.
fn reply_fits(questions: &[Question], reply: &QuestionReply) -> bool {
    match reply {
        QuestionReply::Declined => true,
        QuestionReply::Answered { answers } => {
            answers.len() == questions.len()
                && questions
                    .iter()
                    .zip(answers)
                    .all(|(question, answer)| answer_fits(question, answer))
        }
    }
}

/// An answer as a call's result keeps it: its options in the question's order,
/// and the owner's words without the white space around them.
fn settled(mut answer: QuestionAnswer) -> QuestionAnswer {
    answer.selected.sort_unstable();
    answer.own_words = answer.own_words.map(|words| words.trim().to_owned());
    answer
}

/// The labels of the options that an answer chose, in their order.
fn chosen_labels<'a>(question: &'a Question, answer: &QuestionAnswer) -> Vec<&'a str> {
    answer
        .selected
        .iter()
        .filter_map(|option| usize::try_from(*option).ok())
        .filter_map(|option| question.options.get(option))
        .map(|option| option.label.as_str())
        .collect()
}

/// The result of a call of `ask_owner` with the owner's reply. Only the core
/// writes it, from the checked reply and its own copy of the questions.
pub(crate) fn done(questions: &[Question], reply: QuestionReply) -> Done {
    let (outcome, output) = match reply {
        QuestionReply::Declined => (
            ToolOutcome::QuestionDeclined,
            texts::QUESTION_DECLINED.to_owned(),
        ),
        QuestionReply::Answered { answers } => {
            let answers: Vec<QuestionAnswer> = answers.into_iter().map(settled).collect();
            let shown: Vec<String> = questions
                .iter()
                .zip(&answers)
                .map(|(question, answer)| {
                    texts::question_answer(
                        &question.text,
                        &chosen_labels(question, answer),
                        answer.own_words.as_deref(),
                    )
                })
                .collect();
            (
                ToolOutcome::QuestionAnswered { answers },
                texts::question_answered(&shown),
            )
        }
    };
    Done {
        outcome,
        output,
        archives: Vec::new(),
        images: Vec::new(),
        cited: Vec::new(),
    }
}

/// Whether a call holds the owner's reply to its questions, which stays in the
/// model's context, so that the model does not ask again (ADR-0058,
/// decision 7).
pub(crate) fn holds_reply(call: &ToolCallDocument) -> bool {
    call.result.as_ref().is_some_and(|result| {
        matches!(
            result.outcome,
            ToolOutcome::QuestionAnswered { .. } | ToolOutcome::QuestionDeclined
        )
    })
}

/// The lines of a call that the owner answered in the transcript of a
/// compaction request, whole and without the 500 characters that other calls
/// get, as the owner's messages are. The model wrote the questions and the
/// owner gave the answers, so each stays with its speaker. `None` for any
/// other call.
pub(crate) fn compaction_lines(call: &ToolCallDocument) -> Option<Vec<String>> {
    let result = call.result.as_ref()?;
    let ToolOutcome::QuestionAnswered { answers } = &result.outcome else {
        return None;
    };
    let lines = call
        .questions
        .iter()
        .zip(answers)
        .flat_map(|(question, answer)| {
            [
                texts::compaction_question(&question.text),
                texts::compaction_answer(
                    &chosen_labels(question, answer),
                    answer.own_words.as_deref(),
                ),
            ]
        })
        .collect();
    Some(lines)
}

/// The questions that a run waits for a reply to.
#[derive(Debug)]
pub(crate) struct PendingQuestions {
    part_id: PartId,
    questions: Vec<Question>,
    reply: oneshot::Sender<QuestionReply>,
}

/// A run's wait for a reply. Dropping it, when the run stops waiting for any
/// reason, withdraws the questions.
#[derive(Debug)]
pub(crate) struct WaitingForReply {
    runtime: Runtime,
    conversation: ConversationId,
    part_id: PartId,
}

impl Drop for WaitingForReply {
    fn drop(&mut self) {
        let mut questions = lock(&self.runtime.inner.questions);
        let is_this_call = questions
            .get(&self.conversation)
            .is_some_and(|pending| pending.part_id == self.part_id);
        if is_this_call {
            questions.remove(&self.conversation);
        }
    }
}

impl Runtime {
    /// Registers a run's questions to the owner and returns the wait for the
    /// reply.
    pub(crate) fn open_questions(
        &self,
        conversation: &ConversationId,
        part_id: &PartId,
        questions: &[Question],
    ) -> (WaitingForReply, oneshot::Receiver<QuestionReply>) {
        let (reply, receiver) = oneshot::channel();
        lock(&self.inner.questions).insert(
            conversation.clone(),
            PendingQuestions {
                part_id: part_id.clone(),
                questions: questions.to_vec(),
                reply,
            },
        );
        let waiting = WaitingForReply {
            runtime: self.clone(),
            conversation: conversation.clone(),
            part_id: part_id.clone(),
        };
        (waiting, receiver)
    }

    /// The owner's reply to questions that a run waits for. A part names one
    /// call, whose questions never change, so unlike an approval a reply
    /// names no digest. A reply that does not fit is refused, and the
    /// questions keep waiting.
    pub fn answer_question(&self, input: AnswerQuestion) -> Result<(), ToolsError> {
        let mut questions = lock(&self.inner.questions);
        let pending = questions
            .get(&input.conversation_id)
            .filter(|pending| pending.part_id == input.part_id)
            .ok_or(ToolsError::QuestionNotPending)?;
        if !reply_fits(&pending.questions, &input.reply) {
            return Err(ToolsError::AnswerNotValid);
        }
        let pending = questions
            .remove(&input.conversation_id)
            .ok_or(ToolsError::QuestionNotPending)?;
        pending
            .reply
            .send(input.reply)
            .map_err(|_stopped| ToolsError::QuestionNotPending)?;
        tracing::info!(
            conversation = %input.conversation_id,
            part = %input.part_id,
            "the owner replied to a question"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use mannie_types::tools::{
        MAX_OPTION_DESCRIPTION_CHARS, MAX_OPTION_LABEL_CHARS, MAX_OPTIONS, MAX_OWN_WORDS_BYTES,
        MAX_QUESTION_CHARS, MAX_QUESTIONS, MIN_OPTIONS, Question, QuestionAnswer, QuestionOption,
        QuestionReply, ToolOutcome,
    };
    use serde_json::{Value, json};

    use super::{
        ASK_OWNER, compaction_lines, definition, done, holds_reply, parse_call, reply_fits,
    };

    /// Every object of a schema, at every level, with its path.
    fn objects<'a>(path: &str, schema: &'a Value, found: &mut Vec<(String, &'a Value)>) {
        if schema["type"] == json!("object") {
            found.push((path.to_owned(), schema));
            for (name, property) in schema["properties"].as_object().unwrap() {
                objects(&format!("{path}.{name}"), property, found);
            }
        }
        if schema["type"] == json!("array") {
            objects(&format!("{path}[]"), &schema["items"], found);
        }
    }

    #[test]
    fn the_schema_is_strict_at_every_level() {
        let tool = definition();
        let mut found = Vec::new();
        objects("arguments", &tool.parameters, &mut found);

        assert_eq!(tool.name, ASK_OWNER);
        assert!(tool.strict);
        assert_eq!(found.len(), 3);
        for (path, object) in found {
            let mut required: Vec<&str> = object["required"]
                .as_array()
                .unwrap_or_else(|| panic!("{path} requires nothing"))
                .iter()
                .map(|name| name.as_str().unwrap())
                .collect();
            let mut names: Vec<&str> = object["properties"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            required.sort_unstable();
            names.sort_unstable();
            assert_eq!(required, names, "{path}");
            assert_eq!(object["additionalProperties"], json!(false), "{path}");
        }
    }

    #[test]
    fn the_descriptions_state_the_limits_that_the_schema_cannot() {
        let tool = definition();
        let question = &tool.parameters["properties"]["questions"];
        let options = &question["items"]["properties"]["options"];

        let stated = |schema: &Value, limit: String| {
            let text = schema["description"].as_str().unwrap();
            assert!(text.contains(&limit), "{text} does not say {limit}");
        };
        let option = &options["items"]["properties"];
        stated(question, format!("1 to {MAX_QUESTIONS}"));
        stated(
            &question["items"]["properties"]["question"],
            format!("{MAX_QUESTION_CHARS} characters"),
        );
        stated(options, format!("{MIN_OPTIONS} to {MAX_OPTIONS}"));
        stated(
            &option["label"],
            format!("{MAX_OPTION_LABEL_CHARS} characters"),
        );
        stated(
            &option["description"],
            format!("{MAX_OPTION_DESCRIPTION_CHARS} characters"),
        );
    }

    fn option(label: &str) -> Value {
        json!({ "label": label, "description": "" })
    }

    fn arguments(questions: &[Value]) -> String {
        json!({ "questions": questions }).to_string()
    }

    fn question(text: &str, options: &[Value]) -> Value {
        json!({ "question": text, "multiple": false, "options": options })
    }

    #[test]
    fn a_call_is_checked_and_its_texts_are_cleaned() {
        let parsed = parse_call(&arguments(&[json!({
            "question": "  Which\ndatabase \u{202e}should it use?  ",
            "multiple": true,
            "options": [
                { "label": " PostgreSQL ", "description": " It has the\tJSON queries. " },
                { "label": "SQLite", "description": "   " },
            ],
        })]));

        assert_eq!(
            parsed,
            Ok(vec![Question {
                text: "Which database should it use?".to_owned(),
                multiple: true,
                options: vec![
                    QuestionOption {
                        label: "PostgreSQL".to_owned(),
                        description: Some("It has the JSON queries.".to_owned()),
                    },
                    QuestionOption {
                        label: "SQLite".to_owned(),
                        description: None,
                    },
                ],
            }])
        );
    }

    #[test]
    fn a_call_outside_the_limits_is_refused_and_says_why() {
        let good = question("Which?", &[option("A"), option("B")]);
        let long_question = "x".repeat(1_001);
        let long_label = "x".repeat(101);
        let long_description = "x".repeat(301);
        let cases = [
            ("not json".to_owned(), "expected"),
            (arguments(&[]), "1 to 4 questions"),
            (
                arguments(&[
                    good.clone(),
                    good.clone(),
                    good.clone(),
                    good.clone(),
                    good.clone(),
                ]),
                "1 to 4 questions",
            ),
            (
                arguments(&[question(" \u{200b} ", &[option("A"), option("B")])]),
                "a question is empty",
            ),
            (
                arguments(&[question(&long_question, &[option("A"), option("B")])]),
                "longer than 1000",
            ),
            (arguments(&[question("Which?", &[option("A")])]), "2 to 6"),
            (
                arguments(&[question(
                    "Which?",
                    &[
                        option("A"),
                        option("B"),
                        option("C"),
                        option("D"),
                        option("E"),
                        option("F"),
                        option("G"),
                    ],
                )]),
                "2 to 6",
            ),
            (
                arguments(&[question("Which?", &[option(" "), option("B")])]),
                "label is empty",
            ),
            (
                arguments(&[question("Which?", &[option(&long_label), option("B")])]),
                "longer than 100",
            ),
            (
                arguments(&[question(
                    "Which?",
                    &[
                        json!({ "label": "A", "description": long_description }),
                        option("B"),
                    ],
                )]),
                "longer than 300",
            ),
            (
                arguments(&[question("Which?", &[option("A"), option(" A\n")])]),
                "twice",
            ),
            (
                json!({ "questions": [good], "extra": 1 }).to_string(),
                "unknown field",
            ),
            (
                r#"{"questions":[{"question":"Which?"}]}"#.to_owned(),
                "multiple",
            ),
        ];

        for (input, expected) in cases {
            let refusal = parse_call(&input).unwrap_err();
            assert!(refusal.contains(expected), "{input}: {refusal}");
        }
    }

    #[test]
    fn four_questions_of_six_options_at_their_limits_are_accepted() {
        let label = "x".repeat(100);
        let description = "y".repeat(300);
        let options: Vec<Value> = (0..6)
            .map(|number| {
                let label = format!("{number}{}", &label[1..]);
                json!({ "label": label, "description": description })
            })
            .collect();
        let question = question(&"z".repeat(1_000), &options);

        let parsed = parse_call(&arguments(&[
            question.clone(),
            question.clone(),
            question.clone(),
            question,
        ]))
        .unwrap();

        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[3].options.len(), 6);
    }

    fn asked(multiple: bool) -> Question {
        Question {
            text: "Which extras?".to_owned(),
            multiple,
            options: vec![
                QuestionOption {
                    label: "Caching".to_owned(),
                    description: None,
                },
                QuestionOption {
                    label: "Metrics".to_owned(),
                    description: None,
                },
            ],
        }
    }

    fn answer(selected: &[u32], own_words: Option<&str>) -> QuestionAnswer {
        QuestionAnswer {
            selected: selected.to_vec(),
            own_words: own_words.map(str::to_owned),
        }
    }

    fn fits(question: Question, answer: QuestionAnswer) -> bool {
        reply_fits(
            &[question],
            &QuestionReply::Answered {
                answers: vec![answer],
            },
        )
    }

    #[test]
    fn one_choice_takes_exactly_one_answer() {
        assert!(fits(asked(false), answer(&[1], None)));
        assert!(fits(asked(false), answer(&[], Some("Neither"))));
        assert!(!fits(asked(false), answer(&[], None)));
        assert!(!fits(asked(false), answer(&[0, 1], None)));
        assert!(!fits(asked(false), answer(&[0], Some("And this"))));
    }

    #[test]
    fn several_choices_take_at_least_one_answer() {
        assert!(fits(asked(true), answer(&[1, 0], None)));
        assert!(fits(asked(true), answer(&[0], Some("And tracing"))));
        assert!(fits(asked(true), answer(&[], Some("Only tracing"))));
        assert!(!fits(asked(true), answer(&[], None)));
    }

    #[test]
    fn a_forged_answer_does_not_fit() {
        assert!(!fits(asked(true), answer(&[2], None)));
        assert!(!fits(asked(true), answer(&[u32::MAX], None)));
        assert!(!fits(asked(true), answer(&[0, 0], None)));
        assert!(!fits(asked(true), answer(&[0, 1, 1], None)));
        assert!(!fits(asked(true), answer(&[], Some("  \n "))));
        let too_long = "x".repeat(MAX_OWN_WORDS_BYTES + 1);
        assert!(!fits(asked(true), answer(&[], Some(&too_long))));
        let longest = "x".repeat(MAX_OWN_WORDS_BYTES);
        assert!(fits(asked(true), answer(&[], Some(&longest))));
        let two_questions = [asked(false), asked(false)];
        assert!(!reply_fits(
            &two_questions,
            &QuestionReply::Answered {
                answers: vec![answer(&[0], None)],
            }
        ));
        assert!(reply_fits(&two_questions, &QuestionReply::Declined));
    }

    #[test]
    fn the_model_reads_each_question_with_its_answer() {
        let questions = [asked(false), asked(true), asked(true)];

        let answered = done(
            &questions,
            QuestionReply::Answered {
                answers: vec![
                    answer(&[0], None),
                    answer(&[1, 0], Some("  and tracing\n")),
                    answer(&[], Some("only tracing")),
                ],
            },
        );

        assert_eq!(
            answered.output,
            "The owner answered.\n\n\
             Question: Which extras?\nAnswer: Caching\n\n\
             Question: Which extras?\nAnswer: Caching; Metrics\nIn the owner's own words: and tracing\n\n\
             Question: Which extras?\nIn the owner's own words: only tracing"
        );
        assert_eq!(
            answered.outcome,
            ToolOutcome::QuestionAnswered {
                answers: vec![
                    answer(&[0], None),
                    answer(&[0, 1], Some("and tracing")),
                    answer(&[], Some("only tracing")),
                ],
            }
        );
    }

    #[test]
    fn a_reply_that_declines_says_so_and_keeps_its_place_in_the_context() {
        let declined = done(&[asked(false)], QuestionReply::Declined);

        assert_eq!(declined.outcome, ToolOutcome::QuestionDeclined);
        assert!(declined.output.contains("chose not to answer"));
    }

    fn call_with(outcome: ToolOutcome) -> mannie_types::tools::ToolCallDocument {
        let mut call = mannie_types::tools::ToolCallDocument::receiving(
            "call_1".to_owned(),
            ASK_OWNER.to_owned(),
        );
        call.questions = vec![asked(true)];
        call.result = Some(mannie_types::tools::ToolResult {
            outcome,
            output: "The result.".to_owned(),
            archives: Vec::new(),
            images: Vec::new(),
        });
        call
    }

    #[test]
    fn a_reply_stays_in_the_context_and_other_results_do_not_count_as_one() {
        assert!(holds_reply(&call_with(ToolOutcome::QuestionDeclined)));
        assert!(holds_reply(&call_with(ToolOutcome::QuestionAnswered {
            answers: vec![answer(&[0], None)],
        })));
        assert!(!holds_reply(&call_with(ToolOutcome::Unknown)));
        let mut unfinished = call_with(ToolOutcome::Unknown);
        unfinished.result = None;
        assert!(!holds_reply(&unfinished));
    }

    #[test]
    fn compaction_keeps_a_long_answer_whole_with_each_line_for_its_speaker() {
        let words = "w".repeat(2_000);
        let call = call_with(ToolOutcome::QuestionAnswered {
            answers: vec![answer(&[1], Some(&words))],
        });

        let lines = compaction_lines(&call).unwrap();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Assistant asked the owner: Which extras?");
        assert!(lines[1].starts_with("Owner: Metrics"), "{}", lines[1]);
        assert!(lines[1].ends_with(&words));
        assert_eq!(
            compaction_lines(&call_with(ToolOutcome::QuestionDeclined)),
            None
        );
    }
}
