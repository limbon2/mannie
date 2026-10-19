//! The activity of a tool call (docs/features/system-tools/tool-calls.md, the
//! activity of a call): a few words in which the model says what a call does,
//! such as "Reading package.json", for the owner to read in the reply.
//!
//! The runtime adds the argument to every schema that a model receives
//! ([`with_activity`]) and takes it out of a call's arguments before the tool
//! reads them ([`split_activity`]), so no tool sees it. An activity is the
//! model's claim about its call: the UI shows it as content, and nothing that
//! authorizes a call reads it.

// The cleaning of an activity, which tasks' titles share (mannie_types::labels).
pub use mannie_types::labels::clean_label;

use serde_json::{Map, Value, json};

use crate::texts;

/// The argument that carries a call's activity. No tool may have an argument
/// of this name ([`crate::ToolRegistry::register`]).
pub const ACTIVITY_ARGUMENT: &str = "activity";

/// The most characters of an activity that a call keeps.
pub const MAX_ACTIVITY_CHARS: usize = 100;

/// A call's arguments with the activity taken out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitArguments {
    /// The activity, cleaned for display, or `None` when the call has none.
    pub activity: Option<String>,
    /// The arguments that the tool reads.
    pub arguments: String,
}

/// The schema of a tool's arguments as a model receives it: with the required
/// string [`ACTIVITY_ARGUMENT`], and as strict as the tool's own schema.
pub fn with_activity(parameters: &Value) -> Value {
    let mut schema = parameters.clone();
    let Value::Object(root) = &mut schema else {
        return schema;
    };
    if let Value::Object(properties) = root
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()))
    {
        properties.insert(
            ACTIVITY_ARGUMENT.to_owned(),
            json!({ "type": "string", "description": texts::ACTIVITY }),
        );
    }
    let required = root
        .entry("required")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(names) = required
        && !names.iter().any(|name| name == ACTIVITY_ARGUMENT)
    {
        names.push(Value::String(ACTIVITY_ARGUMENT.to_owned()));
        // The tools' schemas list their required names in order.
        names.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    }
    schema
}

/// Splits a call's arguments into its activity and the arguments that the
/// tool reads. Arguments that are not a JSON object, or that carry no
/// activity, stay as the model wrote them, so that the tool's own parser
/// says what is wrong with them. An activity that is not a string is dropped.
pub fn split_activity(arguments: &str) -> SplitArguments {
    let unchanged = || SplitArguments {
        activity: None,
        arguments: arguments.to_owned(),
    };
    let Ok(Value::Object(mut object)) = serde_json::from_str::<Value>(arguments) else {
        return unchanged();
    };
    let Some(value) = object.remove(ACTIVITY_ARGUMENT) else {
        return unchanged();
    };
    let activity = match value {
        Value::String(text) => clean(&text),
        _ => None,
    };
    SplitArguments {
        activity,
        arguments: Value::Object(object).to_string(),
    }
}

/// An activity as the owner reads it ([`clean_label`] with
/// [`MAX_ACTIVITY_CHARS`]).
fn clean(text: &str) -> Option<String> {
    clean_label(text, MAX_ACTIVITY_CHARS)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::{Value, json};

    use super::{
        ACTIVITY_ARGUMENT, MAX_ACTIVITY_CHARS, SplitArguments, split_activity, with_activity,
    };
    use crate::texts;

    /// Arguments of a tool that refuses unknown fields, as every tool does.
    #[derive(Debug, PartialEq, Eq, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ReadArguments {
        path: String,
    }

    fn strict(properties: &Value, required: &[&str]) -> Value {
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    }

    fn activity_of(arguments: &str) -> Option<String> {
        split_activity(arguments).activity
    }

    #[test]
    fn every_schema_gets_a_required_activity_and_stays_strict() {
        let path = json!({ "type": "string", "description": "The file." });
        let schema = strict(&json!({ "path": path }), &["path"]);

        let offered = with_activity(&schema);

        assert_eq!(
            offered,
            strict(
                &json!({
                    "activity": { "type": "string", "description": texts::ACTIVITY },
                    "path": path,
                }),
                &["activity", "path"],
            )
        );
    }

    #[test]
    fn a_schema_without_arguments_gets_the_activity_alone() {
        let offered = with_activity(&strict(&json!({}), &[]));

        assert_eq!(offered["properties"].as_object().unwrap().len(), 1);
        assert_eq!(offered["required"], json!([ACTIVITY_ARGUMENT]));
        assert_eq!(offered["additionalProperties"], false);
    }

    #[test]
    fn the_tool_reads_its_arguments_without_the_activity() {
        let split = split_activity(r#"{"activity":"Reading the notes","path":"notes.md"}"#);

        assert_eq!(split.activity.as_deref(), Some("Reading the notes"));
        let read: ReadArguments = serde_json::from_str(&split.arguments).unwrap();
        assert_eq!(
            read,
            ReadArguments {
                path: "notes.md".to_owned()
            }
        );
    }

    #[test]
    fn arguments_without_an_activity_stay_as_the_model_wrote_them() {
        for arguments in [
            r#"{ "path" : "notes.md" }"#,
            "",
            "null",
            "[1, 2]",
            r#"{"path": "notes.md""#,
        ] {
            assert_eq!(
                split_activity(arguments),
                SplitArguments {
                    activity: None,
                    arguments: arguments.to_owned(),
                },
                "{arguments}"
            );
        }
    }

    #[test]
    fn an_activity_that_is_not_text_is_dropped() {
        let split = split_activity(r#"{"activity":{"text":"Reading"},"path":"notes.md"}"#);

        assert_eq!(split.activity, None);
        assert_eq!(split.arguments, r#"{"path":"notes.md"}"#);
    }

    #[test]
    fn an_activity_reads_as_one_line_of_visible_text() {
        assert_eq!(
            activity_of(r#"{"activity":"  Reading\n\tthe   notes\u0007 "}"#).as_deref(),
            Some("Reading the notes")
        );
        // A right-to-left override would show "Reading txt.exe" as "Reading exe.txt".
        assert_eq!(
            activity_of("{\"activity\":\"Reading \u{202E}exe.txt\u{202C}\"}").as_deref(),
            Some("Reading exe.txt")
        );
        assert_eq!(
            activity_of("{\"activity\":\"Read\u{200B}ing\u{FEFF} notes\"}").as_deref(),
            Some("Reading notes")
        );
        // Some scripts need the zero-width joiners.
        assert_eq!(
            activity_of("{\"activity\":\"می\u{200C}خوانم\"}").as_deref(),
            Some("می\u{200C}خوانم")
        );
    }

    #[test]
    fn an_activity_without_visible_text_is_none() {
        for activity in ["", "   ", "\u{202E}\u{200B}", "\n\t"] {
            let arguments = json!({ "activity": activity, "path": "notes.md" }).to_string();
            let split = split_activity(&arguments);

            assert_eq!(split.activity, None, "{activity:?}");
            assert_eq!(split.arguments, r#"{"path":"notes.md"}"#);
        }
    }

    #[test]
    fn a_long_activity_is_cut_at_a_character() {
        let long = "é".repeat(MAX_ACTIVITY_CHARS * 2);
        let arguments = json!({ "activity": long }).to_string();

        let activity = activity_of(&arguments).unwrap();

        assert_eq!(activity.chars().count(), MAX_ACTIVITY_CHARS);
        assert!(activity.ends_with('…'), "{activity}");
        let exact = "a".repeat(MAX_ACTIVITY_CHARS);
        assert_eq!(
            activity_of(&json!({ "activity": exact }).to_string()),
            Some(exact)
        );
    }
}
