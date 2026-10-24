//! Tool sources (ADR-0043, decision 2): tools that a plugin publishes while
//! it runs, such as the tools of the tool servers that the owner registered
//! with it.
//!
//! A plugin contributes a source when it starts, and the core takes the
//! source's stream of sets once. Each [`ToolSet`] is the whole list of the
//! source's tools at that moment and replaces the one before. A run reads the
//! sets with its snapshot when it starts and keeps them (ADR-0030, decision
//! 8), so a set that arrives during a reply reaches the next one. A stream
//! that ends empties the source's set, and the source's tools leave with the
//! plugin's other contributions when it stops.
//!
//! The core checks each published tool alone with [`check_published`] and
//! leaves out one that breaks a rule, with a log line, while the rest of its
//! set stays. A published tool also gives way to any tool of the core, a task
//! tool, an executor's tool or a static plugin tool of the same name, so a
//! source never makes another plugin's start fail. A plugin applies the same
//! checks before it publishes, because it does not learn what the core left
//! out.
//!
//! A source's ID follows the rule `Owned`: the plugin's ID, or the plugin's
//! ID, a dot and more (ADR-0034, decision 6).

use std::fmt;
use std::sync::Arc;

use crate::BoxStream;
use crate::contributions::{ExtensionPoint, IdRule};
use crate::ids::PluginId;
use crate::points::tools::{PluginTool, ToolSpec, is_tool_name};

/// The point of plugins' tool sources, whose entries are named by their IDs.
#[derive(Debug)]
pub enum ToolSources {}

impl ExtensionPoint for ToolSources {
    const NAME: &'static str = "tool-sources";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Owned;
    type Entry = PluginToolSource;

    fn id(entry: &PluginToolSource) -> &str {
        &entry.id
    }
}

/// The most tools of one source that the core keeps at a time. A set with
/// more keeps its first ones.
pub const MAX_SOURCE_TOOLS: usize = 64;

/// The most bytes of a published tool's description.
pub const MAX_PUBLISHED_DESCRIPTION_BYTES: usize = 8 * 1024;

/// The most bytes of a published tool's schema, as JSON text.
pub const MAX_PUBLISHED_SCHEMA_BYTES: usize = 64 * 1024;

/// A source of tools that a plugin publishes while it runs.
pub trait ToolSource: Send + Sync + fmt::Debug + 'static {
    /// The sets of the source's tools, each the whole list that replaces the
    /// one before. The core takes the stream once, when the plugin starts,
    /// and drops it when the plugin stops.
    fn sets(&self) -> BoxStream<'static, ToolSet>;
}

/// A plugin's tool source, under its ID.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PluginToolSource {
    /// The plugin's ID, or the plugin's ID, a dot and more.
    pub id: String,
    /// The source.
    pub source: Arc<dyn ToolSource>,
}

impl PluginToolSource {
    /// A source under `id`.
    pub fn new(id: impl Into<String>, source: Arc<dyn ToolSource>) -> Self {
        Self {
            id: id.into(),
            source,
        }
    }
}

/// The whole list of a source's tools at one moment.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ToolSet {
    /// The tools, in the order in which the model is told of them.
    pub tools: Vec<PluginTool>,
}

impl ToolSet {
    /// A set of `tools`.
    pub fn new(tools: Vec<PluginTool>) -> Self {
        Self { tools }
    }
}

/// Why the core leaves a published tool out.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PublishRefusal {
    /// The name does not start with the plugin's prefix and an underscore.
    #[error("the tool name {name:?} does not start with {prefix:?}")]
    Prefix {
        /// The tool's name.
        name: String,
        /// The prefix that the name needs.
        prefix: String,
    },
    /// The name is not lower-case snake case of at most 64 characters.
    #[error("the tool name {name:?} is not lower-case snake case of at most 64 characters")]
    Name {
        /// The tool's name.
        name: String,
    },
    /// The schema has an argument named `activity`, which every call
    /// carries for the owner.
    #[error(
        "the tool {name} has an argument named activity, which every call carries for the owner"
    )]
    Activity {
        /// The tool's name.
        name: String,
    },
    /// The schema is not a JSON object.
    #[error("the schema of the tool {name} is not a JSON object")]
    Schema {
        /// The tool's name.
        name: String,
    },
    /// The description or the schema is longer than a published tool's may
    /// be.
    #[error("the description or the schema of the tool {name} is too long")]
    TooLong {
        /// The tool's name.
        name: String,
    },
}

/// Checks a tool that the plugin `plugin` publishes, as the core checks it:
/// a name of the plugin's prefix, an underscore and more, in lower-case snake
/// case of at most 64 characters; a schema that is a JSON object without an
/// argument named `activity`; and a description and a schema within their
/// limits.
///
/// Whether another tool has the name is the core's to know; such a tool wins
/// over the published one.
pub fn check_published(plugin: &PluginId, spec: &ToolSpec) -> Result<(), PublishRefusal> {
    let name = spec.name.clone();
    let prefix = format!("{}_", plugin.prefix());
    if !spec.name.starts_with(&prefix) || spec.name.len() == prefix.len() {
        return Err(PublishRefusal::Prefix { name, prefix });
    }
    if !is_tool_name(&spec.name) {
        return Err(PublishRefusal::Name { name });
    }
    let Some(schema) = spec.parameters.as_object() else {
        return Err(PublishRefusal::Schema { name });
    };
    let takes_activity = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|properties| properties.contains_key("activity"));
    if takes_activity {
        return Err(PublishRefusal::Activity { name });
    }
    let schema_bytes =
        serde_json::to_string(&spec.parameters).map_or(usize::MAX, |text| text.len());
    if spec.description.len() > MAX_PUBLISHED_DESCRIPTION_BYTES
        || schema_bytes > MAX_PUBLISHED_SCHEMA_BYTES
    {
        return Err(PublishRefusal::TooLong { name });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        MAX_PUBLISHED_DESCRIPTION_BYTES, MAX_PUBLISHED_SCHEMA_BYTES, PublishRefusal,
        check_published,
    };
    use crate::ids::PluginId;
    use crate::points::tools::ToolSpec;

    fn spec(name: &str, parameters: serde_json::Value) -> ToolSpec {
        ToolSpec::new(name, "Does a thing.", parameters).with_loose_schema()
    }

    #[test]
    fn a_published_tool_is_named_after_its_plugin_in_snake_case() {
        let plugin = PluginId::parse("log-viewer").unwrap();
        let object = json!({ "type": "object" });

        assert_eq!(
            check_published(&plugin, &spec("log_viewer_tail", object.clone())),
            Ok(())
        );
        for name in ["tail", "log-viewer_tail", "log_viewer_", "log_viewertail"] {
            assert!(
                matches!(
                    check_published(&plugin, &spec(name, object.clone())),
                    Err(PublishRefusal::Prefix { .. })
                ),
                "{name}"
            );
        }
        assert!(matches!(
            check_published(&plugin, &spec("log_viewer_Tail", object.clone())),
            Err(PublishRefusal::Name { .. })
        ));
        let long = format!("log_viewer_{}", "a".repeat(60));
        assert!(matches!(
            check_published(&plugin, &spec(&long, object)),
            Err(PublishRefusal::Name { .. })
        ));
    }

    #[test]
    fn a_published_schema_is_an_object_without_the_activity_and_within_its_limits() {
        let plugin = PluginId::parse("mcp").unwrap();

        assert!(matches!(
            check_published(&plugin, &spec("mcp_find", json!(true))),
            Err(PublishRefusal::Schema { .. })
        ));
        assert!(matches!(
            check_published(
                &plugin,
                &spec(
                    "mcp_find",
                    json!({ "type": "object", "properties": { "activity": { "type": "string" } } })
                )
            ),
            Err(PublishRefusal::Activity { .. })
        ));
        let long_description = ToolSpec::new(
            "mcp_find",
            "a".repeat(MAX_PUBLISHED_DESCRIPTION_BYTES + 1),
            json!({ "type": "object" }),
        );
        assert!(matches!(
            check_published(&plugin, &long_description),
            Err(PublishRefusal::TooLong { .. })
        ));
        let long_schema =
            json!({ "type": "object", "description": "a".repeat(MAX_PUBLISHED_SCHEMA_BYTES) });
        assert!(matches!(
            check_published(&plugin, &spec("mcp_find", long_schema)),
            Err(PublishRefusal::TooLong { .. })
        ));
    }
}
