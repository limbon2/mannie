//! The tools of a server as agents are offered them (plugins/mcp/docs/
//! README.md, tools for agents): each under its name for the model, with the
//! server's description and the model's copy of its schema, checked as the
//! core checks a published tool (ADR-0043, decision 1), so the plugin knows
//! which tools the core takes and the settings say why one is left out.

use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::points::tool_sources::{PublishRefusal, check_published};
use mannie_plugin_sdk::points::tools::ToolSpec;

use crate::commands::ToolProblem;
use crate::naming::model_names;
use crate::schema::{SchemaRefusal, model_schema};
use crate::store::{ServerRecord, ToolRecord};
use crate::texts;

/// A tool as agents are offered it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OfferedTool {
    /// Its server.
    pub(crate) server_id: String,
    /// The tool, as the server names it.
    pub(crate) tool: String,
    /// Whether the owner turned it on.
    pub(crate) enabled: bool,
    /// What the model is told of it.
    pub(crate) spec: ToolSpec,
}

/// A tool of a server, offered, or left out with its name for the model and
/// the reason.
pub(crate) type Offer<'a> = (&'a ToolRecord, Result<OfferedTool, (String, ToolProblem)>);

/// A server's tools, each offered or left out, in the server's order.
pub(crate) fn offered_tools<'a>(
    plugin: &PluginId,
    record: &ServerRecord,
    tools: &'a [ToolRecord],
) -> Vec<Offer<'a>> {
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    let model_names = model_names(&record.prefix, &names);
    tools
        .iter()
        .zip(model_names)
        .map(|(tool, model_name)| {
            let offered = offer(plugin, record, tool, &model_name)
                .map_err(|problem| (model_name.clone(), problem));
            (tool, offered)
        })
        .collect()
}

/// One tool, as agents are offered it.
fn offer(
    plugin: &PluginId,
    record: &ServerRecord,
    tool: &ToolRecord,
    model_name: &str,
) -> Result<OfferedTool, ToolProblem> {
    let model = model_schema(&tool.schema).map_err(|refusal| match refusal {
        SchemaRefusal::NotAnObject => ToolProblem::SchemaInvalid,
        SchemaRefusal::ActivityClash => ToolProblem::ActivityClash,
    })?;
    let spec =
        ToolSpec::new(model_name, description(record, tool), model.schema).with_loose_schema();
    check_published(plugin, &spec).map_err(|refusal| match refusal {
        PublishRefusal::TooLong { .. } => ToolProblem::TooLong,
        _other => ToolProblem::SchemaInvalid,
    })?;
    Ok(OfferedTool {
        server_id: record.id.clone(),
        tool: tool.name.clone(),
        enabled: tool.enabled,
        spec,
    })
}

/// What the model reads of a tool: the server's description, else its
/// title, else a line that names the server.
fn description(record: &ServerRecord, tool: &ToolRecord) -> String {
    if !tool.description.trim().is_empty() {
        return tool.description.clone();
    }
    match &tool.title {
        Some(title) if !title.trim().is_empty() => title.clone(),
        _none => texts::no_description(&record.name),
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::ids::PluginId;
    use serde_json::json;

    use super::offered_tools;
    use crate::commands::ToolProblem;
    use crate::store::{ServerRecord, StoredEndpoint, ToolRecord};

    fn record() -> ServerRecord {
        ServerRecord {
            id: "0199e0f2-0000-7000-8000-0000000000aa".to_owned(),
            name: "GitHub".to_owned(),
            prefix: "github".to_owned(),
            endpoint: StoredEndpoint::Url {
                url: "https://mcp.example.com/mcp".to_owned(),
            },
            plain: Vec::new(),
            secret_names: Vec::new(),
            secret_parts: 0,
            missing_secrets: Vec::new(),
            oauth: false,
            client_id: None,
            client_secret_set: false,
            signed_in: false,
            oauth_parts: 0,
            enabled: true,
            images: true,
            instructions: true,
            timeout_seconds: 300,
            era: None,
            listed: true,
        }
    }

    fn tool(name: &str, schema: serde_json::Value) -> ToolRecord {
        ToolRecord {
            name: name.to_owned(),
            position: 0,
            title: None,
            description: String::new(),
            schema,
            hints: Vec::new(),
            enabled: true,
            new: false,
        }
    }

    #[test]
    fn a_tool_is_offered_under_its_model_name_or_left_out_with_its_reason() {
        let plugin = PluginId::parse("mcp").unwrap();
        let tools = [
            tool(
                "create-issue",
                json!({ "type": "object", "properties": { "activity": {} } }),
            ),
            tool("broken", json!({ "type": "string" })),
            tool("big", serde_json::Value::Null),
            tool(
                "clash",
                json!({ "type": "object", "properties": { "activity": {}, "activity_value": {} } }),
            ),
        ];

        let offered = offered_tools(&plugin, &record(), &tools);

        let first = offered[0].1.as_ref().unwrap();
        assert_eq!(first.spec.name, "mcp_github_create_issue");
        assert!(!first.spec.strict);
        assert!(
            first.spec.parameters["properties"]
                .get("activity_value")
                .is_some()
        );
        assert!(first.spec.description.contains("GitHub"));
        let problems: Vec<Option<ToolProblem>> = offered
            .iter()
            .map(|(_tool, offered)| offered.as_ref().err().map(|(_name, problem)| *problem))
            .collect();
        assert_eq!(
            problems,
            [
                None,
                Some(ToolProblem::SchemaInvalid),
                Some(ToolProblem::SchemaInvalid),
                Some(ToolProblem::ActivityClash),
            ]
        );
    }
}
