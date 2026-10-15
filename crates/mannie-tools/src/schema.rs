//! The JSON Schemas of tool arguments (docs/features/system-tools/
//! tools-and-results.md, arguments and schemas).
//!
//! Each tool's arguments are a Rust type, and its schema is generated from the
//! type, so the schema and the parser cannot disagree. The schemas sent to
//! models are self-contained and work in strict mode: every property is
//! required, optional ones accept `null`, no other properties are allowed, and
//! keywords that strict mode refuses are left out. The field documentation of
//! an argument type is what the model reads about each property.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};

/// Keywords that strict mode refuses or that only add noise. Limits that they
/// would state are in the property descriptions instead.
const REMOVED_KEYWORDS: [&str; 14] = [
    "$schema",
    "title",
    "format",
    "pattern",
    "default",
    "examples",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
];

/// The schema of a tool's arguments.
pub(crate) fn parameters<T: JsonSchema>() -> Value {
    let mut settings = SchemaSettings::draft2020_12();
    settings.inline_subschemas = true;
    settings.meta_schema = None;
    let mut schema = settings
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value();
    if let Value::Object(root) = &mut schema {
        // The tool's own description goes with the tool, not its arguments.
        root.remove("description");
        if !root.contains_key("properties") {
            root.insert("properties".to_owned(), Value::Object(Map::new()));
        }
    }
    make_strict(&mut schema);
    schema
}

/// Makes a schema and its subschemas strict.
fn make_strict(schema: &mut Value) {
    let Value::Object(map) = schema else {
        return;
    };
    for keyword in REMOVED_KEYWORDS {
        map.remove(keyword);
    }
    if let Some(Value::String(description)) = map.get_mut("description") {
        *description = unwrap_lines(description);
    }
    let names = match map.get_mut("properties") {
        Some(Value::Object(properties)) => {
            properties.values_mut().for_each(make_strict);
            Some(
                properties
                    .keys()
                    .map(|name| Value::String(name.clone()))
                    .collect::<Vec<_>>(),
            )
        }
        _ => None,
    };
    if let Some(names) = names {
        map.insert("required".to_owned(), Value::Array(names));
        map.insert("additionalProperties".to_owned(), Value::Bool(false));
    }
    if let Some(items) = map.get_mut("items") {
        make_strict(items);
    }
    for keyword in ["anyOf", "oneOf", "allOf"] {
        if let Some(Value::Array(subschemas)) = map.get_mut(keyword) {
            subschemas.iter_mut().for_each(make_strict);
        }
    }
}

/// A doc comment's text with the line breaks of its wrapping removed:
/// lines join with a space, and paragraphs stay apart.
fn unwrap_lines(text: &str) -> String {
    text.split("\n\n")
        .map(|paragraph| {
            paragraph
                .lines()
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use schemars::JsonSchema;
    use serde::Deserialize;
    use serde_json::json;

    use super::{parameters, unwrap_lines};

    /// Arguments of a made-up tool.
    #[derive(Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    #[expect(dead_code, reason = "only the schema of the type is tested")]
    struct Arguments {
        /// The pattern to match.
        pattern: String,
        /// How many to return.
        #[serde(default)]
        limit: Option<u32>,
        /// The changes.
        edits: Vec<Edit>,
    }

    /// One change.
    #[derive(Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    #[expect(dead_code, reason = "only the schema of the type is tested")]
    struct Edit {
        /// The text to replace.
        old_text: String,
    }

    #[test]
    fn wrapped_doc_comments_read_as_sentences() {
        assert_eq!(
            unwrap_lines("The folder. The workspace when\nnull.\n\nA second paragraph."),
            "The folder. The workspace when null.\n\nA second paragraph."
        );
    }

    #[test]
    fn schemas_are_strict_and_self_contained() {
        assert_eq!(
            parameters::<Arguments>(),
            json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "The pattern to match." },
                    "limit": { "type": ["integer", "null"], "description": "How many to return." },
                    "edits": {
                        "type": "array",
                        "description": "The changes.",
                        "items": {
                            "type": "object",
                            "description": "One change.",
                            "properties": {
                                "old_text": { "type": "string", "description": "The text to replace." }
                            },
                            "required": ["old_text"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["edits", "limit", "pattern"],
                "additionalProperties": false
            })
        );
    }
}
