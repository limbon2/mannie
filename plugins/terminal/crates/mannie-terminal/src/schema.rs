//! The JSON Schema of a tool's arguments, generated from the type that
//! reads them, so the two cannot disagree. Models that run tools in strict
//! mode, such as OpenAI's, need every property required, optional ones
//! nullable, no other properties, and none of the keywords that strict mode
//! refuses, as the core's own tools have them (docs/features/system-tools/
//! tools-and-results.md, arguments and schemas). The field documentation is
//! what the model reads about each property.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};

/// Keywords that strict mode refuses or that only add noise. Limits go into
/// the property descriptions instead.
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

/// Makes a schema and its properties strict.
fn make_strict(schema: &mut Value) {
    let Value::Object(map) = schema else {
        return;
    };
    for keyword in REMOVED_KEYWORDS {
        map.remove(keyword);
    }
    if let Some(Value::String(description)) = map.get_mut("description") {
        *description = description.split_whitespace().collect::<Vec<_>>().join(" ");
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
        _none => None,
    };
    if let Some(names) = names {
        map.insert("required".to_owned(), Value::Array(names));
        map.insert("additionalProperties".to_owned(), Value::Bool(false));
    }
}
