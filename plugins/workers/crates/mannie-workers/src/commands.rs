//! The commands of the workers feature (plugins/workers/docs/
//! pairing-and-link.md, contracts): pairing a worker, naming one, removing
//! one, following the paired workers, and their errors. Each command and the
//! channel are specs of the plugin SDK (ADR-0034, decision 8), so the host
//! checks their input before the plugin's handler runs, answers
//! `plugin_off` while Workers is off and ends the channel when it stops.

use mannie_plugin_sdk::contracts::WorkerId;
use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, InputRefusal, Interface, PluginError, Validate,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::view::{WorkerListUpdate, WorkerView};

/// The most characters of a worker's name, as a worker's configuration
/// allows them (plugins/workers/docs/service.md).
pub const MAX_WORKER_NAME_CHARACTERS: usize = 64;

/// The argument of `pair_worker`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PairWorker {
    /// The host's name or address, with the port when it is not 7445.
    pub address: String,
    /// The code that `mannie-worker pair` printed on the host.
    pub code: String,
    /// The name the owner gives the worker in Mannie. Without one, or with a
    /// blank one, Mannie uses the name in the worker's configuration.
    #[serde(default)]
    pub name: Option<String>,
}

impl Validate for PairWorker {
    /// The hub checks the address, the code and the name, and answers with
    /// codes of its own that say what the owner can correct.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `rename_worker`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameWorker {
    /// The worker.
    pub worker_id: WorkerId,
    /// The name the owner gives the worker in Mannie. A blank name goes back
    /// to the name in the worker's configuration.
    pub name: String,
}

impl Validate for RenameWorker {
    /// The hub checks the name, and answers `name_invalid` with its limit.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `remove_worker`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerRef {
    /// The worker.
    pub worker_id: WorkerId,
}

impl Validate for WorkerRef {
    /// A worker ID decodes only in its form; an unknown one is
    /// `worker_not_found`.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// Checks a name that the owner gives a worker and returns it without
/// surrounding white space, or `None` for a blank name, which leaves the
/// worker with the name in its configuration. Line breaks and other control
/// characters become spaces.
pub fn check_worker_name(name: &str) -> Result<Option<String>, WorkersError> {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_WORKER_NAME_CHARACTERS {
        return Err(WorkersError::NameInvalid {
            max_characters: u32::try_from(MAX_WORKER_NAME_CHARACTERS).unwrap_or(u32::MAX),
        });
    }
    Ok(Some(trimmed.to_owned()))
}

/// Why a command about workers failed. The host's refusals, such as
/// `plugin_off`, come beside these (ADR-0034, decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkersError {
    /// The address is not a host name or address with an optional port.
    #[error("the address is not valid")]
    AddressInvalid,
    /// Nothing answered at the address.
    #[error("the worker cannot be reached")]
    Unreachable,
    /// Something answered at the address that is not a Mannie worker.
    #[error("the address does not belong to a Mannie worker")]
    NotAWorker,
    /// The worker refused the code: it is wrong, expired or used.
    #[error("the worker refused the code")]
    CodeRefused,
    /// The code allowed its attempts; the worker deleted it.
    #[error("the code allowed its attempts")]
    TooManyAttempts,
    /// The worker is paired with another master.
    #[error("the worker is paired already")]
    AlreadyPaired,
    /// This master is paired with the worker already.
    #[error("this worker was added already")]
    AlreadyAdded,
    /// The worker presented another key than the one this master pinned.
    #[error("the worker's key changed")]
    KeyChanged,
    /// The worker runs a version of Mannie that this one does not work with.
    #[error("the worker's version {version} does not work with this one")]
    Incompatible {
        /// The worker's version.
        version: String,
    },
    /// No paired worker has this ID.
    #[error("the worker is not paired")]
    WorkerNotFound,
    /// The name that the owner gave a worker is longer than Mannie accepts.
    #[error("a worker's name has at most {max_characters} characters")]
    NameInvalid {
        /// The limit.
        max_characters: u32,
    },
    /// Another paired worker shows this name, ignoring case.
    #[error("another worker has this name")]
    NameTaken,
    /// The operating system's credential store, which keeps the master's
    /// key, cannot be used.
    #[error("the credential store cannot be used")]
    CredentialStoreUnavailable,
    /// The store failed. The log has the cause.
    #[error("the worker could not be saved")]
    StorageFailed,
}

impl PluginError for WorkersError {
    /// The stable code of the error, for logs.
    fn code(&self) -> &'static str {
        match self {
            Self::AddressInvalid => "workers.address_invalid",
            Self::Unreachable => "workers.unreachable",
            Self::NotAWorker => "workers.not_a_worker",
            Self::CodeRefused => "workers.code_refused",
            Self::TooManyAttempts => "workers.too_many_attempts",
            Self::AlreadyPaired => "workers.already_paired",
            Self::AlreadyAdded => "workers.already_added",
            Self::KeyChanged => "workers.key_changed",
            Self::Incompatible { .. } => "workers.incompatible",
            Self::WorkerNotFound => "workers.worker_not_found",
            Self::NameInvalid { .. } => "workers.name_invalid",
            Self::NameTaken => "workers.name_taken",
            Self::CredentialStoreUnavailable => "workers.credential_store_unavailable",
            Self::StorageFailed => "workers.storage_failed",
        }
    }
}

/// `pair_worker`: pairs with a worker.
#[derive(Debug)]
pub enum PairWorkerCommand {}

impl CommandSpec for PairWorkerCommand {
    const NAME: &'static str = "pair_worker";
    const DOCS: &'static str = "Pairs with the worker at the input's address with the code that `mannie-worker pair` printed on its host.";
    type Input = PairWorker;
    type Output = WorkerView;
    type Error = WorkersError;
}

/// `rename_worker`: names a worker.
#[derive(Debug)]
pub enum RenameWorkerCommand {}

impl CommandSpec for RenameWorkerCommand {
    const NAME: &'static str = "rename_worker";
    const DOCS: &'static str = "Gives a worker the name that the owner chose in Mannie, or with a blank name goes back to the name in the worker's configuration.";
    type Input = RenameWorker;
    type Output = WorkerView;
    type Error = WorkersError;
}

/// `remove_worker`: removes a worker.
#[derive(Debug)]
pub enum RemoveWorkerCommand {}

impl CommandSpec for RemoveWorkerCommand {
    const NAME: &'static str = "remove_worker";
    const DOCS: &'static str =
        "Removes a worker: it is unpaired when it can be reached, and the master forgets it.";
    type Input = WorkerRef;
    type Output = ();
    type Error = WorkersError;
}

/// `subscribe_workers`: follows the paired workers.
#[derive(Debug)]
pub enum SubscribeWorkersChannel {}

impl ChannelSpec for SubscribeWorkersChannel {
    const NAME: &'static str = "subscribe_workers";
    const DOCS: &'static str = "Follows the paired workers: the list first, then the list after each change of a worker or its status, until Workers stops.";
    type Input = ();
    type Item = WorkerListUpdate;
    type Error = WorkersError;
}

/// The plugin's commands and channel, and the limit that the UI shows.
pub fn interface() -> Interface {
    Interface::new()
        .command::<PairWorkerCommand>()
        .command::<RenameWorkerCommand>()
        .command::<RemoveWorkerCommand>()
        .channel::<SubscribeWorkersChannel>()
        .constant(
            "MAX_WORKER_NAME_CHARACTERS",
            "The most characters of a worker's name.",
            u32::try_from(MAX_WORKER_NAME_CHARACTERS).unwrap_or(u32::MAX),
        )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::ipc::PluginError;

    use super::{MAX_WORKER_NAME_CHARACTERS, PairWorker, WorkersError, check_worker_name};

    #[test]
    fn a_worker_name_is_trimmed_and_cleaned_and_a_blank_one_means_none() {
        assert_eq!(
            check_worker_name("  freebsd-01 ").unwrap().as_deref(),
            Some("freebsd-01")
        );
        assert_eq!(
            check_worker_name("build\n01").unwrap().as_deref(),
            Some("build 01")
        );
        assert_eq!(check_worker_name(" \t\n").unwrap(), None);
        assert_eq!(check_worker_name("").unwrap(), None);
    }

    #[test]
    fn a_worker_name_has_at_most_64_characters() {
        let longest = "é".repeat(MAX_WORKER_NAME_CHARACTERS);
        assert_eq!(
            check_worker_name(&longest).unwrap().as_deref(),
            Some(longest.as_str())
        );
        assert_eq!(
            check_worker_name(&format!("{longest}x")),
            Err(WorkersError::NameInvalid { max_characters: 64 })
        );
    }

    #[test]
    fn pairing_without_a_name_reads_as_before_and_an_unknown_field_is_refused() {
        let input: PairWorker =
            serde_json::from_value(serde_json::json!({ "address": "build-01.lan", "code": "X" }))
                .unwrap();
        let unknown = serde_json::from_value::<PairWorker>(
            serde_json::json!({ "address": "build-01.lan", "code": "X", "port": 7445 }),
        );

        assert_eq!(input.name, None);
        assert!(unknown.is_err(), "{unknown:?}");
    }

    #[test]
    fn errors_serialize_with_their_codes() {
        assert_eq!(
            serde_json::to_value(WorkersError::CodeRefused).unwrap(),
            serde_json::json!({ "code": "code_refused" })
        );
        assert_eq!(
            serde_json::to_value(WorkersError::Incompatible {
                version: "0.2.0".to_owned()
            })
            .unwrap(),
            serde_json::json!({ "code": "incompatible", "version": "0.2.0" })
        );
        assert_eq!(WorkersError::KeyChanged.code(), "workers.key_changed");
        assert_eq!(
            serde_json::to_value(WorkersError::NameInvalid { max_characters: 64 }).unwrap(),
            serde_json::json!({ "code": "name_invalid", "maxCharacters": 64 })
        );
        assert_eq!(
            serde_json::to_value(WorkersError::NameTaken).unwrap(),
            serde_json::json!({ "code": "name_taken" })
        );
    }
}
