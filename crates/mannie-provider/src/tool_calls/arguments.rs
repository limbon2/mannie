//! Schema-guided repair of Qwen4-Max's stringified scalar arguments. This
//! runs before the runtime persists, validates and authorizes the call
//! (ADR-0011, decision 5). It never makes an invalid call valid by defaulting
//! a value, and never interprets text in a string-valued argument as JSON.

use serde_json::Value;

pub(super) fn normalize(arguments: &str, schema: &Value) -> Option<String> {
    let mut value: Value = serde_json::from_str(arguments).ok()?;

    if !value.is_object() {
        return None;
    }

    let changed = normalize_value(&mut value, schema);
    changed.then(|| value.to_string())
}

fn normalize_value(value: &mut Value, schema: &Value) -> bool {
    // Do not guess which branch applies or follow references from a tool
    // server. Validation still reports values that this narrow repair skips.
    let has_branches = ["$ref", "anyOf", "oneOf", "allOf", "not", "if"]
        .iter()
        .any(|keyword| schema.get(keyword).is_some());

    if has_branches {
        return false;
    }

    match value {
        Value::Object(values) => {
            let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
                return false;
            };

            let mut changed = false;

            for (name, value) in values {
                if let Some(property) = properties.get(name) {
                    changed |= normalize_value(value, property);
                }
            }
            changed
        }
        Value::Array(values) => {
            if schema.get("prefixItems").is_some() {
                return false;
            }

            let Some(items) = schema.get("items").filter(|items| items.is_object()) else {
                return false;
            };

            let mut changed = false;

            for value in values {
                changed |= normalize_value(value, items);
            }
            changed
        }
        Value::String(text) => {
            let Some(kind) = scalar_type(schema) else {
                return false;
            };

            let Ok(parsed) = serde_json::from_str::<Value>(text) else {
                return false;
            };

            let matches = match kind {
                "boolean" => parsed.is_boolean(),
                "integer" => parsed.is_i64() || parsed.is_u64(),
                "number" => parsed.is_number(),
                _ => false,
            };

            if matches {
                *value = parsed;
            }

            matches
        }
        _ => false,
    }
}

/// One scalar type, optionally nullable. Unions that accept strings or more
/// than one non-null type are left alone.
fn scalar_type(schema: &Value) -> Option<&str> {
    let kind = match schema.get("type")? {
        Value::String(kind) => kind.as_str(),
        Value::Array(types) => {
            let mut kinds = types.iter().filter(|kind| kind.as_str() != Some("null"));
            let kind = kinds.next()?.as_str()?;

            if kinds.next().is_some() {
                return None;
            }

            kind
        }
        _ => return None,
    };
    matches!(kind, "boolean" | "integer" | "number").then_some(kind)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::{Value, json};

    use super::normalize;

    fn parameters(property: &Value) -> Value {
        json!({"type": "object", "properties": {"value": property}})
    }

    #[test]
    fn nullable_scalars_accept_only_json_values_of_the_declared_type() {
        for (kind, text, expected) in [
            ("integer", "200", json!(200)),
            ("integer", "-1", json!(-1)),
            ("integer", "18446744073709551615", json!(u64::MAX)),
            ("number", "1.5", json!(1.5)),
            ("number", "1e2", json!(100.0)),
            ("boolean", "false", json!(false)),
            ("boolean", "true", json!(true)),
        ] {
            let schema = parameters(&json!({"type": [kind, "null"]}));
            let raw = json!({"value": text}).to_string();
            let normalized = normalize(&raw, &schema).unwrap();
            let value: Value = serde_json::from_str(&normalized).unwrap();

            assert_eq!(value, json!({"value": expected}), "{kind}: {text}");
        }
    }

    #[test]
    fn invalid_or_ambiguous_scalars_are_not_guessed() {
        for (kind, texts) in [
            (
                "integer",
                vec!["1.5", "1e2", "01", "", "null", "18446744073709551616"],
            ),
            ("boolean", vec!["False", "0", "1", "yes", "", "null"]),
            ("number", vec!["NaN", "Infinity", "1e999", "null"]),
            ("string", vec!["200", "false", "null"]),
            ("object", vec!["{}"]),
            ("array", vec!["[]"]),
        ] {
            for text in texts {
                let schema = parameters(&json!({"type": [kind, "null"]}));
                let raw = json!({"value": text}).to_string();

                assert_eq!(normalize(&raw, &schema), None, "{kind}: {text}");
            }
        }
        for schema in [
            json!({"type": ["string", "integer", "null"]}),
            json!({"type": ["integer", "boolean"]}),
            json!({"anyOf": [{"type": "integer"}, {"type": "string"}]}),
            json!({"type": "integer", "$ref": "#/$defs/value"}),
            json!({}),
        ] {
            assert_eq!(normalize(r#"{"value":"200"}"#, &parameters(&schema)), None);
        }
    }

    #[test]
    fn nested_values_follow_only_their_own_property_and_item_schemas() {
        let schema = json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "line": {"type": "integer"},
                            "text": {"type": "string"}
                        }
                    }
                },
                "flags": {"type": "array", "items": {"type": "boolean"}},
                "limit": {"type": ["integer", "null"]},
                "union": {"type": ["string", "integer"]}
            }
        });
        let raw = json!({
            "edits": [{"line": "2", "text": "200"}],
            "flags": ["false", true],
            "limit": null,
            "union": "200",
            "unknown": "false"
        });
        let normalized = normalize(&raw.to_string(), &schema).unwrap();
        let value: Value = serde_json::from_str(&normalized).unwrap();

        assert_eq!(
            value,
            json!({
                "edits": [{"line": 2, "text": "200"}],
                "flags": [false, true],
                "limit": null,
                "union": "200",
                "unknown": "false"
            })
        );
    }

    #[test]
    fn unchanged_or_malformed_arguments_keep_the_original_bytes() {
        let schema = parameters(&json!({"type": ["integer", "null"]}));

        for raw in [
            r#"{ "value" : 200 }"#,
            r#"{"value":null}"#,
            r#"{"other":"200"}"#,
            r#"{"value":"200""#,
            r#"["200"]"#,
            "null",
        ] {
            assert_eq!(normalize(raw, &schema), None, "{raw}");
        }
    }

    #[test]
    fn tuple_items_do_not_use_the_schema_of_later_items() {
        let schema = parameters(&json!({
            "type": "array",
            "prefixItems": [{"type": "string"}],
            "items": {"type": "integer"}
        }));

        assert_eq!(normalize(r#"{"value":["200","300"]}"#, &schema), None);
    }

    #[test]
    fn normalized_values_still_need_argument_validation() {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Arguments {
            value: u32,
        }

        let schema = parameters(&json!({"type": "integer"}));
        let valid = normalize(r#"{"value":"200"}"#, &schema).unwrap();
        assert_eq!(
            serde_json::from_str::<Arguments>(&valid).unwrap().value,
            200
        );

        for raw in [
            r#"{"value":"-1"}"#,
            r#"{"value":"4294967296"}"#,
            r#"{"value":"200","extra":true}"#,
        ] {
            let normalized = normalize(raw, &schema).unwrap();
            assert!(serde_json::from_str::<Arguments>(&normalized).is_err());
        }
    }
}
