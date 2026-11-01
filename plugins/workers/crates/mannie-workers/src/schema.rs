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

/// Makes a schema and its properties strict, the items of its arrays and
/// the branches of its choices too.
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
    if let Some(items) = map.get_mut("items") {
        make_strict(items);
    }
    for choice in ["anyOf", "oneOf"] {
        if let Some(Value::Array(branches)) = map.get_mut(choice) {
            branches.iter_mut().for_each(make_strict);
        }
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

#[cfg(test)]
mod tests {
    use schemars::JsonSchema;
    use serde::Deserialize;
    use serde_json::json;

    use super::parameters;

    /// Lines of a file.
    #[derive(Debug, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    #[expect(
        dead_code,
        reason = "the type exists for its schema, which the test reads"
    )]
    struct Lines {
        /// The file.
        path: String,
        /// The first line; the first of the file when null.
        start: Option<u32>,
    }

    /// The arguments of a test tool.
    #[derive(Debug, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    #[expect(
        dead_code,
        reason = "the type exists for its schema, which the test reads"
    )]
    struct Arguments {
        /// The   folder,
        /// with spaces.
        folder: Option<String>,
        /// Ranges of lines.
        lines: Vec<Lines>,
    }

    #[test]
    fn every_property_is_required_and_nested_objects_are_strict_too() {
        let schema = parameters::<Arguments>();

        assert_eq!(schema["required"], json!(["folder", "lines"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(
            schema["properties"]["folder"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(
            schema["properties"]["folder"]["description"],
            json!("The folder, with spaces.")
        );
        let item = &schema["properties"]["lines"]["items"];
        assert_eq!(item["required"], json!(["path", "start"]));
        assert_eq!(item["additionalProperties"], json!(false));
        assert!(schema.get("$schema").is_none());
        assert!(schema.get("description").is_none());
    }
}
