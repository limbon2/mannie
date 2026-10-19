//! What a worker says about its host in `Hello` (plugins/workers/docs/
//! host-and-ceiling.md): its facts, its ceiling and its versions. The worker
//! writes all of it, so the master treats it as content. The master keeps it
//! with the worker and shows it in the Workers plugin's views.

use mannie_plugin_sdk::contracts::CeilingMode;
use serde::{Deserialize, Serialize};
use specta::Type;

/// What a worker discovers about its host when it starts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostFacts {
    /// The operating system, its version and the architecture, as
    /// `system_info` reports them.
    pub system: String,
    /// The host's name.
    pub hostname: String,
    /// The shells on the host, the one that runs commands first.
    #[serde(default)]
    pub shells: Vec<String>,
    /// The programs of a fixed list that the host has on its `PATH`.
    #[serde(default)]
    pub tools: Vec<String>,
}

/// A worker's ceiling as the master shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CeilingView {
    /// The mode.
    pub mode: CeilingMode,
    /// The allowed folders.
    #[serde(default)]
    pub allowed_folders: Vec<String>,
    /// The host's command rules, each as its words, with the folder it is
    /// limited to when it has one.
    #[serde(default)]
    pub rules: Vec<String>,
    /// Whether a conversation in full access on the master may answer the
    /// host's requests (ADR-0029, decision 5). True when the worker does not
    /// say, as the configuration's default is.
    #[serde(default = "accepts_by_default")]
    pub accepts_master_full_access: bool,
}

fn accepts_by_default() -> bool {
    true
}

/// What the master keeps of what a worker said about itself when it last
/// connected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkerDetails {
    /// What the host is for.
    #[serde(default)]
    pub purpose: String,
    /// The host's facts.
    #[serde(default)]
    pub facts: Option<HostFacts>,
    /// The worker's ceiling.
    #[serde(default)]
    pub ceiling: Option<CeilingView>,
    /// The worker's version of Mannie.
    #[serde(default)]
    pub version: Option<String>,
    /// How many tasks the worker runs at once.
    #[serde(default)]
    pub max_parallel_tasks: u32,
}
