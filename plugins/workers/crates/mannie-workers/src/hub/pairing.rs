//! Pairing from the master (plugins/workers/docs/pairing-and-link.md,
//! pairing): the owner types the host's address, the code that
//! `mannie-worker pair` printed and, if they like, a name for the worker.
//! The master connects with its key and records the worker's key from the
//! handshake; the worker proves first that it knows the code for this
//! session's keys, then the master does, and both sides pin each other's
//! keys.

use mannie_link::host::WorkerDetails;
use mannie_link::keys::KeyPin;
use mannie_link::pairing::{Code, Session, master_proof, nonce, verify_worker_proof};
use mannie_link::proto::pairing::pairing_service_client::PairingServiceClient;
use mannie_link::proto::pairing::{
    BeginRequest, FinishRequest, RefusalReason, begin_response, finish_response,
};
use mannie_link::tls;
use mannie_plugin_sdk::contracts::WorkerId;
use tonic::Code as StatusCode;
use tonic::transport::Channel;

use super::connection::{ConnectError, Incompatible, channel, hello};
use super::{Hub, lock};
use crate::commands::{PairWorker, WorkersError, check_worker_name};
use crate::store::{NewPairedWorker, PairedWorker, WorkersStoreError};
use crate::view::{WorkerStatus, WorkerView};

/// The worker's port when the address names none.
pub const DEFAULT_PORT: u16 = 7445;

/// The most characters of an address.
const MAX_ADDRESS_CHARS: usize = 255;

/// An address as the hub connects to it: `host:port`, with the default port
/// when it names none. A host is a name or an IPv4 address, or an IPv6
/// address in brackets.
pub fn normalize_address(text: &str) -> Result<String, WorkersError> {
    let text = text.trim();
    let valid_characters = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if text.is_empty() || text.len() > MAX_ADDRESS_CHARS || !valid_characters {
        return Err(WorkersError::AddressInvalid);
    }
    let (host, port) = if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or(WorkersError::AddressInvalid)?;
        let port = match after {
            "" => None,
            port => Some(port.strip_prefix(':').ok_or(WorkersError::AddressInvalid)?),
        };
        (format!("[{host}]"), port)
    } else {
        match text.split_once(':') {
            Some((host, port)) => (host.to_owned(), Some(port)),
            None => (text.to_owned(), None),
        }
    };
    let bare_host = host.trim_start_matches('[').trim_end_matches(']');
    if bare_host.is_empty() || host.contains(':') && !host.starts_with('[') {
        return Err(WorkersError::AddressInvalid);
    }
    let port = match port {
        None => DEFAULT_PORT,
        Some(port) => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or(WorkersError::AddressInvalid)?,
    };
    Ok(format!("{host}:{port}"))
}

fn refusal_error(reason: RefusalReason) -> WorkersError {
    match reason {
        RefusalReason::AlreadyPaired => WorkersError::AlreadyPaired,
        RefusalReason::TooManyAttempts => WorkersError::TooManyAttempts,
        RefusalReason::NoValidCode | RefusalReason::InvalidProof | RefusalReason::Unspecified => {
            WorkersError::CodeRefused
        }
    }
}

fn call_error(status: &tonic::Status) -> WorkersError {
    match status.code() {
        // Something answered that does not serve pairing.
        StatusCode::Unimplemented | StatusCode::InvalidArgument => WorkersError::NotAWorker,
        _other => WorkersError::Unreachable,
    }
}

fn new_worker_id() -> WorkerId {
    WorkerId::parse(&uuid::Uuid::now_v7().to_string())
        .expect("the uuid crate writes UUIDs in their hyphenated form")
}

/// Both proofs over one connection: the worker proves first that it knows
/// the code for this session's keys, then the master does. Returns the name
/// that the worker gave in its challenge.
async fn exchange_proofs(
    channel: Channel,
    code: &Code,
    master_pin: &KeyPin,
    worker_pin: &KeyPin,
) -> Result<String, WorkersError> {
    let master_nonce = nonce().map_err(|_random| WorkersError::Unreachable)?;
    let mut client = PairingServiceClient::new(channel);
    let begun = client
        .begin(BeginRequest {
            master_nonce: master_nonce.to_vec(),
        })
        .await
        .map_err(|status| call_error(&status))?
        .into_inner();
    let challenge = match begun.outcome {
        Some(begin_response::Outcome::Challenge(challenge)) => challenge,
        Some(begin_response::Outcome::Refusal(refusal)) => {
            return Err(refusal_error(refusal.reason()));
        }
        None => return Err(WorkersError::NotAWorker),
    };
    let session = Session {
        master_nonce: &master_nonce,
        worker_nonce: &challenge.worker_nonce,
        master_pin,
        worker_pin,
    };
    // A worker that does not know the code, or a relay in the middle with a
    // key of its own, never makes a proof that verifies here.
    if !verify_worker_proof(code, &session, &challenge.worker_proof) {
        tracing::warn!(
            worker = worker_pin.short(),
            "a worker's pairing proof did not verify"
        );
        return Err(WorkersError::CodeRefused);
    }
    let finished = client
        .finish(FinishRequest {
            master_proof: master_proof(code, &session),
        })
        .await
        .map_err(|status| call_error(&status))?
        .into_inner();
    match finished.outcome {
        Some(finish_response::Outcome::Paired(_paired)) => Ok(challenge.name),
        Some(finish_response::Outcome::Refusal(refusal)) => Err(refusal_error(refusal.reason())),
        None => Err(WorkersError::NotAWorker),
    }
}

impl Hub {
    /// Pairs with the worker at the input's address with its code, stores
    /// it with the owner's name for it, if any, and starts connecting to it.
    pub async fn pair(&self, input: PairWorker) -> Result<WorkerView, WorkersError> {
        let address = normalize_address(&input.address)?;
        let code = Code::parse(&input.code).ok_or(WorkersError::CodeRefused)?;
        // The name is checked before the worker is contacted, so a refused
        // name never uses up the one-time code.
        let chosen_name = match input.name.as_deref() {
            Some(name) => check_worker_name(name)?,
            None => None,
        };
        if let Some(name) = &chosen_name {
            self.check_name_free(name, None)?;
        }
        let key = self.key().await?;
        let config = tls::client_config(&key, None).map_err(|error| {
            tracing::error!(%error, "could not set up the link's TLS");
            WorkersError::Unreachable
        })?;
        let (channel, worker_pin) = channel(&address, config)
            .await
            .map_err(|_error| WorkersError::Unreachable)?;
        if lock(&self.inner.paired)
            .values()
            .any(|paired| paired.pin == worker_pin)
        {
            return Err(WorkersError::AlreadyAdded);
        }
        let challenge_name =
            exchange_proofs(channel.clone(), &code, &key.pin(), &worker_pin).await?;
        let (name, details, status) = match hello(&channel, &self.inner.version).await {
            Ok(hello) => (hello.name, hello.details, WorkerStatus::Connecting),
            Err(ConnectError::Incompatible(incompatible)) => {
                let Incompatible { name, details, .. } = *incompatible;
                (name, details, WorkerStatus::Incompatible)
            }
            Err(_other) => (
                challenge_name,
                WorkerDetails::default(),
                WorkerStatus::Connecting,
            ),
        };
        let record = self
            .store_paired(NewPairedWorker {
                id: new_worker_id(),
                name,
                chosen_name,
                address,
                key_pin: worker_pin.as_str().to_owned(),
                details,
            })
            .await?;
        tracing::info!(worker = %record.id, pin = worker_pin.short(), "paired with a worker");
        let mut view = self.add(record, worker_pin);
        if status == WorkerStatus::Incompatible {
            view.status = status;
        }
        Ok(view)
    }

    /// Stores a worker that was just paired and reads its record back.
    async fn store_paired(&self, worker: NewPairedWorker) -> Result<PairedWorker, WorkersError> {
        let id = worker.id.clone();
        self.inner
            .store
            .add_paired_worker(worker)
            .await
            .map_err(|error| match error {
                WorkersStoreError::KeyTaken => WorkersError::AlreadyAdded,
                other => {
                    tracing::error!(error = %other, error.code = other.code(), "could not store a paired worker");
                    WorkersError::StorageFailed
                }
            })?;
        self.inner
            .store
            .paired_workers()
            .await
            .map_err(|error| {
                tracing::error!(%error, error.code = error.code(), "could not read the paired workers");
                WorkersError::StorageFailed
            })?
            .into_iter()
            .find(|record| record.id == id)
            .ok_or(WorkersError::StorageFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_address;
    use crate::commands::WorkersError;

    #[test]
    fn an_address_gets_the_default_port_and_bad_ones_are_refused() {
        assert_eq!(
            normalize_address("build-01.lan").unwrap(),
            "build-01.lan:7445"
        );
        assert_eq!(
            normalize_address(" 10.0.0.5:9000 ").unwrap(),
            "10.0.0.5:9000"
        );
        assert_eq!(normalize_address("[fd00::1]").unwrap(), "[fd00::1]:7445");
        assert_eq!(normalize_address("[::1]:7500").unwrap(), "[::1]:7500");
        for bad in [
            "",
            "host:0",
            "host:99999",
            "http://host",
            "host/path",
            "a b",
            "fd00::1",
            "[::1",
            ":7445",
        ] {
            assert_eq!(
                normalize_address(bad),
                Err(WorkersError::AddressInvalid),
                "{bad}"
            );
        }
    }
}
