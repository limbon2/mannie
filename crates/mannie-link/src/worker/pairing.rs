//! Pairing on the worker (docs/features/workers/pairing-and-link.md,
//! pairing): the code that `mannie-worker pair` writes, and the service that
//! checks it.
//!
//! `pair` writes the code to `<data>/pairing-code`, owned by the service's
//! account. The service reads it for each start, counts the attempts, and
//! deletes it once it is used, expired or out of attempts. A start and its
//! finish must come on the same connection, which keeps the master's key and
//! both nonces between them.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mannie_types::clock::{Clock, UtcMillis};
use serde::{Deserialize, Serialize};
use tonic::{Request, Response, Status};

use super::Binding;
use crate::keys::KeyPin;
use crate::pairing::{
    CODE_ATTEMPTS, CODE_LIFETIME, Code, NONCE_BYTES, Session, nonce, verify_master_proof,
    worker_proof,
};
use crate::proto::pairing::{
    BeginRequest, BeginResponse, Challenge, FinishRequest, FinishResponse, Paired, Refusal,
    RefusalReason, begin_response, finish_response, pairing_service_server::PairingService,
};
use crate::tls::Peer;

/// The file name of the pairing code in the data folder.
pub const CODE_FILE: &str = "pairing-code";

/// The code file, as `mannie-worker pair` writes it.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeFile {
    /// The code in its canonical form.
    code: String,
    /// When the code was made, in milliseconds since 1970.
    created_at: i64,
}

/// Writes a new code to the data folder and returns it, for `mannie-worker
/// pair`. `now` is the code's creation time. A code written before is
/// replaced.
pub fn write_code(data: &Path, now: UtcMillis) -> io::Result<Code> {
    let code = Code::generate().map_err(io::Error::other)?;
    let file = CodeFile {
        code: code.canonical().to_owned(),
        created_at: now.as_millis(),
    };
    let text = serde_json::to_vec(&file).map_err(io::Error::other)?;
    super::write_private(&data.join(CODE_FILE), &text)?;
    Ok(code)
}

/// A start whose finish has not come yet.
#[derive(Debug)]
struct Pending {
    master_pin: KeyPin,
    master_nonce: Vec<u8>,
    worker_nonce: Vec<u8>,
}

#[derive(Debug, Default)]
struct State {
    /// Starts by connection.
    pending: HashMap<u64, Pending>,
    /// The attempts of the current code, by the code's creation time.
    attempts: HashMap<i64, u32>,
}

/// The pairing service.
#[derive(Debug, Clone)]
pub(crate) struct Pairing {
    data: PathBuf,
    name: String,
    own_pin: KeyPin,
    binding: Arc<Binding>,
    state: Arc<Mutex<State>>,
    clock: Arc<dyn Clock>,
}

fn refusal(reason: RefusalReason) -> Refusal {
    Refusal {
        reason: reason.into(),
    }
}

impl Pairing {
    pub(crate) fn new(
        data: PathBuf,
        name: String,
        own_pin: KeyPin,
        binding: Arc<Binding>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            data,
            name,
            own_pin,
            binding,
            state: Arc::default(),
            clock,
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn code_path(&self) -> PathBuf {
        self.data.join(CODE_FILE)
    }

    fn forget_code(&self) {
        match std::fs::remove_file(self.code_path()) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(%error, "could not delete the pairing code"),
        }
    }

    /// The current code with its creation time, while it is valid. An
    /// expired code is deleted, and so is a code made after now: the clock
    /// went back, and its age is unknown.
    fn valid_code(&self) -> Option<(Code, i64)> {
        let text = std::fs::read(self.code_path()).ok()?;
        let file: CodeFile = serde_json::from_slice(&text).ok()?;
        let elapsed = self.clock.now().as_millis().saturating_sub(file.created_at);
        let fresh = u64::try_from(elapsed)
            .is_ok_and(|elapsed| Duration::from_millis(elapsed) <= CODE_LIFETIME);
        if !fresh {
            self.forget_code();
            return None;
        }
        Some((Code::parse(&file.code)?, file.created_at))
    }

    fn peer(request: &Request<impl Sized>) -> Result<Peer, Status> {
        request
            .extensions()
            .get::<Peer>()
            .cloned()
            .ok_or_else(|| Status::internal("the connection has no peer"))
    }

    fn start(&self, peer: &Peer, master_nonce: Vec<u8>) -> Result<Challenge, RefusalReason> {
        if self.binding.pin().is_some() {
            return Err(RefusalReason::AlreadyPaired);
        }
        let (code, created_at) = self.valid_code().ok_or(RefusalReason::NoValidCode)?;
        {
            let mut state = self.state();
            let attempts = state.attempts.entry(created_at).or_insert(0);
            if *attempts >= CODE_ATTEMPTS {
                drop(state);
                self.forget_code();
                return Err(RefusalReason::TooManyAttempts);
            }
            *attempts += 1;
        }
        if master_nonce.len() != NONCE_BYTES {
            return Err(RefusalReason::InvalidProof);
        }
        let worker_nonce = nonce().map_err(|_random| RefusalReason::NoValidCode)?;
        let proof = worker_proof(
            &code,
            &Session {
                master_nonce: &master_nonce,
                worker_nonce: &worker_nonce,
                master_pin: &peer.pin,
                worker_pin: &self.own_pin,
            },
        );
        self.state().pending.insert(
            peer.connection,
            Pending {
                master_pin: peer.pin.clone(),
                master_nonce,
                worker_nonce: worker_nonce.to_vec(),
            },
        );
        Ok(Challenge {
            worker_nonce: worker_nonce.to_vec(),
            worker_proof: proof,
            name: self.name.clone(),
        })
    }

    fn complete(&self, peer: &Peer, master_proof: &[u8]) -> Result<Paired, RefusalReason> {
        let pending = self
            .state()
            .pending
            .remove(&peer.connection)
            .ok_or(RefusalReason::InvalidProof)?;
        if pending.master_pin != peer.pin {
            return Err(RefusalReason::InvalidProof);
        }
        if self.binding.pin().is_some() {
            return Err(RefusalReason::AlreadyPaired);
        }
        let (code, _created_at) = self.valid_code().ok_or(RefusalReason::NoValidCode)?;
        let session = Session {
            master_nonce: &pending.master_nonce,
            worker_nonce: &pending.worker_nonce,
            master_pin: &pending.master_pin,
            worker_pin: &self.own_pin,
        };
        if !verify_master_proof(&code, &session, master_proof) {
            return Err(RefusalReason::InvalidProof);
        }
        if let Err(error) = self.binding.set(&pending.master_pin) {
            tracing::error!(%error, "could not keep the pin of the master");
            return Err(RefusalReason::NoValidCode);
        }
        self.forget_code();
        tracing::info!(master = pending.master_pin.short(), "paired with a master");
        Ok(Paired {
            name: self.name.clone(),
        })
    }
}

#[tonic::async_trait]
impl PairingService for Pairing {
    async fn begin(
        &self,
        request: Request<BeginRequest>,
    ) -> Result<Response<BeginResponse>, Status> {
        let peer = Self::peer(&request)?;
        let outcome = match self.start(&peer, request.into_inner().master_nonce) {
            Ok(challenge) => begin_response::Outcome::Challenge(challenge),
            Err(reason) => {
                tracing::info!(
                    master = peer.pin.short(),
                    reason = reason.as_str_name(),
                    "refused to start a pairing"
                );
                begin_response::Outcome::Refusal(refusal(reason))
            }
        };
        Ok(Response::new(BeginResponse {
            outcome: Some(outcome),
        }))
    }

    async fn finish(
        &self,
        request: Request<FinishRequest>,
    ) -> Result<Response<FinishResponse>, Status> {
        let peer = Self::peer(&request)?;
        let outcome = match self.complete(&peer, &request.into_inner().master_proof) {
            Ok(paired) => finish_response::Outcome::Paired(paired),
            Err(reason) => {
                tracing::info!(
                    master = peer.pin.short(),
                    reason = reason.as_str_name(),
                    "refused to finish a pairing"
                );
                finish_response::Outcome::Refusal(refusal(reason))
            }
        };
        Ok(Response::new(FinishResponse {
            outcome: Some(outcome),
        }))
    }
}
