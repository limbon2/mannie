//! The schemas of the servers' tools as the model reads them (plugins/mcp/
//! docs/README.md, tools for agents). A server's schema may use features
//! that providers refuse or that only cost room, so the model's copy:
//!
//! - inlines the references into the schema's own definitions, which some
//!   providers do not follow, to a depth that stops a reference that leads
//!   back to itself;
//! - drops `$schema`, `$id`, `$comment`, the definitions once inlined, and
//!   every keyword that starts with `x-`, such as `x-mcp-header`;
//! - has an object at its root;
//! - renames a top-level property named `activity`, which the core adds to
//!   every call for the owner, and the plugin names it back in the call.
//!
//! The server receives the arguments with its own names.

use serde_json::{Map, Value};

/// The property that the core adds to every call.
const ACTIVITY: &str = "activity";

/// The name that a server's own property `activity` has for the model.
pub(crate) const RENAMED_ACTIVITY: &str = "activity_value";

/// How deep references are inlined. A deeper one becomes an empty schema,
/// which takes any value.
const MAX_REFERENCE_DEPTH: usize = 8;

/// The keywords that the model's copy drops wherever they are.
const DROPPED: [&str; 3] = ["$schema", "$id", "$comment"];

/// The model's copy of a server's schema, and whether its property
/// `activity` was renamed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ModelSchema {
    /// The schema.
    pub(crate) schema: Value,
    /// Whether the server's property `activity` is [`RENAMED_ACTIVITY`] for
    /// the model.
    pub(crate) renamed_activity: bool,
}

/// Why a server's schema cannot be offered to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaRefusal {
    /// The schema's root is not an object schema.
    NotAnObject,
    /// The schema has a property named [`RENAMED_ACTIVITY`] beside
    /// `activity`, so the rename would take a name of its own.
    ActivityClash,
}

/// The model's copy of a server's schema for a tool's arguments.
pub(crate) fn model_schema(schema: &Value) -> Result<ModelSchema, SchemaRefusal> {
    let Value::Object(root) = schema else {
        return Err(SchemaRefusal::NotAnObject);
    };
    let definitions = definitions_of(root);
    let mut copy = inlined(schema, &definitions, 0);
    let Value::Object(object) = &mut copy else {
        return Err(SchemaRefusal::NotAnObject);
    };
    object.remove("$defs");
    object.remove("definitions");
    match object.get("type") {
        None => {
            object.insert("type".to_owned(), Value::String("object".to_owned()));
        }
        Some(Value::String(kind)) if kind == "object" => {}
        Some(_) => return Err(SchemaRefusal::NotAnObject),
    }
    let renamed_activity = rename_activity(object)?;
    Ok(ModelSchema {
        schema: copy,
        renamed_activity,
    })
}

/// The definitions that local references may name: those of `$defs` and of
/// `definitions`, by their reference.
fn definitions_of(root: &Map<String, Value>) -> Vec<(String, Value)> {
    let mut definitions = Vec::new();
    for keyword in ["$defs", "definitions"] {
        if let Some(Value::Object(entries)) = root.get(keyword) {
            for (name, definition) in entries {
                definitions.push((format!("#/{keyword}/{name}"), definition.clone()));
            }
        }
    }
    definitions
}

/// A schema with its local references inlined and the dropped keywords
/// left out, at `depth` references deep.
fn inlined(schema: &Value, definitions: &[(String, Value)], depth: usize) -> Value {
    match schema {
        Value::Object(object) => {
            if let Some(Value::String(reference)) = object.get("$ref") {
                let found = definitions
                    .iter()
                    .find(|(name, _definition)| name == reference)
                    .map(|(_name, definition)| definition);
                return match found {
                    Some(definition) if depth < MAX_REFERENCE_DEPTH => {
                        inlined(definition, definitions, depth + 1)
                    }
                    // A reference elsewhere, or one past the depth, takes
                    // any value, which the server still checks.
                    _ => Value::Object(Map::new()),
                };
            }
            let copy: Map<String, Value> = object
                .iter()
                .filter(|(keyword, _value)| {
                    !DROPPED.contains(&keyword.as_str()) && !keyword.starts_with("x-")
                })
                .map(|(keyword, value)| (keyword.clone(), inlined(value, definitions, depth)))
                .collect();
            Value::Object(copy)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| inlined(item, definitions, depth))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Renames a top-level property named `activity`, in `properties` and in
/// `required`, and says whether it did.
fn rename_activity(object: &mut Map<String, Value>) -> Result<bool, SchemaRefusal> {
    let Some(Value::Object(properties)) = object.get_mut("properties") else {
        return Ok(false);
    };
    let Some(activity) = properties.remove(ACTIVITY) else {
        return Ok(false);
    };
    if properties.contains_key(RENAMED_ACTIVITY) {
        return Err(SchemaRefusal::ActivityClash);
    }
    properties.insert(RENAMED_ACTIVITY.to_owned(), activity);
    if let Some(Value::Array(required)) = object.get_mut("required") {
        for name in required.iter_mut() {
            if name.as_str() == Some(ACTIVITY) {
                *name = Value::String(RENAMED_ACTIVITY.to_owned());
            }
        }
    }
    Ok(true)
}

/// The arguments of a call as the server names them: the model's renamed
/// `activity` back under its own name.
pub(crate) fn server_arguments(
    mut arguments: Map<String, Value>,
    renamed_activity: bool,
) -> Map<String, Value> {
    if renamed_activity && let Some(value) = arguments.remove(RENAMED_ACTIVITY) {
        arguments.insert(ACTIVITY.to_owned(), value);
    }
    arguments
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value, json};

    use super::{RENAMED_ACTIVITY, SchemaRefusal, model_schema, server_arguments};

    #[test]
    fn references_inline_and_the_keywords_that_cost_room_go() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "urn:tool",
            "type": "object",
            "properties": {
                "issue": { "$ref": "#/$defs/issue" },
                "labels": { "type": "array", "items": { "$ref": "#/definitions/label" } },
                "token": { "type": "string", "x-mcp-header": "X-Token" }
            },
            "required": ["issue"],
            "$defs": {
                "issue": {
                    "type": "object",
                    "properties": { "title": { "type": "string", "$comment": "short" } }
                }
            },
            "definitions": { "label": { "type": "string" } }
        });

        let model = model_schema(&schema).unwrap();

        assert_eq!(
            model.schema,
            json!({
                "type": "object",
                "properties": {
                    "issue": { "type": "object", "properties": { "title": { "type": "string" } } },
                    "labels": { "type": "array", "items": { "type": "string" } },
                    "token": { "type": "string" }
                },
                "required": ["issue"]
            })
        );
        assert!(!model.renamed_activity);
    }

    #[test]
    fn a_reference_that_leads_back_to_itself_stops() {
        let schema = json!({
            "type": "object",
            "properties": { "node": { "$ref": "#/$defs/node" } },
            "$defs": {
                "node": {
                    "type": "object",
                    "properties": { "child": { "$ref": "#/$defs/node" } }
                }
            }
        });

        let model = model_schema(&schema).unwrap();

        let mut depth = 0;
        let mut node = &model.schema["properties"]["node"];
        while let Some(child) = node
            .get("properties")
            .and_then(|properties| properties.get("child"))
        {
            depth += 1;
            node = child;
        }
        assert!(depth >= 7, "{depth}");
        assert_eq!(node, &json!({}));
    }

    #[test]
    fn the_root_is_an_object_schema() {
        assert_eq!(
            model_schema(&json!({ "properties": {} })).unwrap().schema,
            json!({ "type": "object", "properties": {} })
        );
        assert_eq!(
            model_schema(&json!({})).unwrap().schema,
            json!({ "type": "object" })
        );
        for schema in [
            json!("object"),
            json!({ "type": "string" }),
            json!(null),
            json!([]),
        ] {
            assert_eq!(
                model_schema(&schema),
                Err(SchemaRefusal::NotAnObject),
                "{schema}"
            );
        }
    }

    #[test]
    fn a_servers_own_activity_is_renamed_and_named_back_in_the_call() {
        let schema = json!({
            "type": "object",
            "properties": { "activity": { "type": "string" }, "id": { "type": "string" } },
            "required": ["activity", "id"]
        });
        let clashing = json!({
            "type": "object",
            "properties": { "activity": {}, "activity_value": {} }
        });

        let model = model_schema(&schema).unwrap();
        let mut arguments = Map::new();
        arguments.insert(RENAMED_ACTIVITY.to_owned(), json!("hiking"));
        arguments.insert("id".to_owned(), json!("7"));

        assert!(model.renamed_activity);
        assert_eq!(model.schema["required"], json!([RENAMED_ACTIVITY, "id"]));
        assert!(model.schema["properties"].get("activity").is_none());
        assert_eq!(
            Value::Object(server_arguments(arguments.clone(), true)),
            json!({ "activity": "hiking", "id": "7" })
        );
        assert_eq!(
            Value::Object(server_arguments(arguments.clone(), false)),
            Value::Object(arguments)
        );
        assert_eq!(model_schema(&clashing), Err(SchemaRefusal::ActivityClash));
    }
}
