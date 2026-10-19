//! A paired worker as the master knows it (plugins/workers/docs/
//! pairing-and-link.md, the connection; worker-activity.md): its status and
//! what it said about itself, for the runtime's delegations and the UI.

use mannie_link::host::WorkerDetails;
use mannie_plugin_sdk::contracts::{UtcMillis, WorkerId};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The state of the master's link to a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    /// The master is connecting.
    Connecting,
    /// The link works.
    Connected,
    /// The worker cannot be reached; the master tries again.
    Offline,
    /// The worker's version is too old, or newer than the master's.
    Incompatible,
    /// The worker presented another key: it was reset or reinstalled, or
    /// something impersonates it.
    KeyChanged,
    /// The worker no longer accepts this master.
    Unpaired,
}

impl WorkerStatus {
    /// The status's name, as the logs write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Offline => "offline",
            Self::Incompatible => "incompatible",
            Self::KeyChanged => "key_changed",
            Self::Unpaired => "unpaired",
        }
    }
}

/// A paired worker. Its configured name and details are what the worker said
/// about itself, and its chosen name is the owner's, so all of them are
/// content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkerView {
    /// The worker.
    pub id: WorkerId,
    /// The name that Mannie and the conversation agent use: the owner's
    /// chosen name, or else the name in the worker's configuration.
    pub name: String,
    /// The name in the worker's configuration, as the worker gave it when
    /// it last connected.
    pub configured_name: String,
    /// The name that the owner gave the worker in Mannie, if any.
    pub chosen_name: Option<String>,
    /// The address that the master connects to.
    pub address: String,
    /// The state of the link.
    pub status: WorkerStatus,
    /// What it said about itself when it last connected.
    pub details: WorkerDetails,
    /// The first 12 characters of the pin of its key.
    pub key_pin: String,
    /// When the master paired with it.
    pub paired_at: UtcMillis,
    /// When the master last connected to it.
    pub connected_at: Option<UtcMillis>,
}

/// A message of the workers channel: the paired workers, first as they are
/// and then after every change. The list stays short, so each change sends
/// all of it. The host ends the channel (ADR-0034, decision 8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkerListUpdate {
    /// The paired workers, in the order of pairing.
    Workers {
        /// The workers.
        workers: Vec<WorkerView>,
    },
}
