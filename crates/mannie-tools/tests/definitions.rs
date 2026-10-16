//! The tool definitions that models receive, compared with a reviewed copy in
//! `tests/fixtures/definitions.json`: a change to a description or a schema
//! changes how models use the tools, so it shows in review.
//!
//! `MANNIE_UPDATE_EXPECTED=1` writes the file from the registry instead of
//! comparing; review the result before committing it.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::path::PathBuf;

use mannie_tools::{PermissionClass, ToolRegistry, with_activity};
use serde_json::{Value, json};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/definitions.json")
}

fn definitions() -> Value {
    let registry = ToolRegistry::with_system_tools();
    Value::Array(
        registry
            .tools()
            .iter()
            .map(|tool| {
                let class = match tool.class() {
                    PermissionClass::ReadOnly => "read_only",
                    PermissionClass::SideEffects => "side_effects",
                };
                json!({
                    "name": tool.name(),
                    "class": class,
                    "description": tool.description(),
                    // As the runtime offers them: with the activity.
                    "parameters": with_activity(tool.parameters()),
                })
            })
            .collect(),
    )
}

#[test]
fn the_definitions_match_their_reviewed_copy() {
    let actual = definitions();
    if std::env::var_os("MANNIE_UPDATE_EXPECTED").is_some() {
        let text = serde_json::to_string_pretty(&actual).unwrap();
        std::fs::write(fixture(), format!("{text}\n")).unwrap();
        return;
    }
    let text = std::fs::read_to_string(fixture()).unwrap();
    let expected: Value = serde_json::from_str(&text).unwrap();

    assert_eq!(
        actual,
        expected,
        "the tool definitions changed; review them and update {}",
        fixture().display()
    );
}

#[test]
fn every_schema_is_strict() {
    let Value::Array(tools) = definitions() else {
        panic!("the definitions are a list");
    };
    for tool in &tools {
        let parameters = &tool["parameters"];
        assert_eq!(parameters["type"], "object", "{tool}");
        assert_eq!(parameters["additionalProperties"], false, "{tool}");
        let properties = parameters["properties"].as_object().unwrap();
        let required = parameters["required"].as_array().unwrap();
        assert_eq!(properties.len(), required.len(), "{tool}");
        assert!(parameters.get("$schema").is_none(), "{tool}");
        assert!(!parameters.to_string().contains("$ref"), "{tool}");
    }
}
