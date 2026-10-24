//! The plugin's tool source (ADR-0043, decision 1): the tools that agents
//! are offered from the servers, published as a new set whenever a server
//! connects, lists other tools, leaves, or the owner switches a tool.
//!
//! The set holds each tool that the owner turned on of each server that is
//! on and connected or connecting, under the checks that the core applies
//! (`offered.rs`), and `mcp_read_resource` while some connected server lists
//! resources. A set that would equal the one before is not sent.

use std::sync::Arc;

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::BoxStream;
use mannie_plugin_sdk::points::tool_sources::{ToolSet, ToolSource};
use mannie_plugin_sdk::points::tools::{AgentSelector, PluginTool};

use crate::offered::OfferedTool;
use crate::resources::ReadResource;
use crate::section::AGENTS;
use crate::servers::Servers;
use crate::tool::ServerTool;

/// The source's ID, which is the plugin's.
pub(crate) const SOURCE_ID: &str = "mcp";

/// The servers' tools, as a source of tools.
#[derive(Debug)]
pub(crate) struct McpToolSource {
    servers: Servers,
}

impl McpToolSource {
    pub(crate) fn new(servers: Servers) -> Self {
        Self { servers }
    }
}

/// What a set holds, to tell whether it changed: each server's tool by its
/// spec, and whether the read tool is there.
#[derive(Debug, Clone, PartialEq)]
struct Contents {
    tools: Vec<OfferedTool>,
    reads: bool,
}

impl ToolSource for McpToolSource {
    fn sets(&self) -> BoxStream<'static, ToolSet> {
        let servers = self.servers.clone();
        let revisions = servers.follow();
        stream::unfold(
            (servers, revisions, None::<Contents>),
            |(servers, mut revisions, last)| async move {
                loop {
                    let contents = contents(&servers);
                    if last.as_ref() != Some(&contents) {
                        let set = set(&servers, &contents);
                        return Some((set, (servers, revisions, Some(contents))));
                    }
                    if revisions.changed().await.is_err() {
                        return None;
                    }
                }
            },
        )
        .boxed()
    }
}

/// What the servers offer now.
fn contents(servers: &Servers) -> Contents {
    let reads = servers
        .connected()
        .iter()
        .any(|server| !server.resources.is_empty() || !server.templates.is_empty());
    Contents {
        tools: servers.offered(),
        reads,
    }
}

/// The set of the tools that `contents` names.
fn set(servers: &Servers, contents: &Contents) -> ToolSet {
    let agents = || AgentSelector::Agents(AGENTS.iter().map(|agent| (*agent).to_owned()).collect());
    let mut tools: Vec<PluginTool> = contents
        .tools
        .iter()
        .map(|offered| {
            let tool = ServerTool::new(
                servers.clone(),
                offered.server_id.clone(),
                offered.tool.clone(),
                offered.spec.clone(),
            );
            PluginTool::new(Arc::new(tool), agents()).with_side_effects()
        })
        .collect();
    if contents.reads {
        tools.push(
            PluginTool::new(Arc::new(ReadResource::new(servers.clone())), agents())
                .with_side_effects(),
        );
    }
    ToolSet::new(tools)
}
