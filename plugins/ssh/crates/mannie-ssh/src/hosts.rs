//! The registered hosts (plugins/ssh/docs/README.md, hosts): the owner's
//! commands, the list that the channel, the section, the mentions and the
//! tool read, and the sessions of the hosts.
//!
//! A host's password lives only in the credential store, as `host/<ID>` of
//! the keyed family `host` (ADR-0036, decision 8). Adding a host stores its
//! row first, without a password, then the password, then marks the row,
//! so no password is ever kept for a host that is not registered. Removing a
//! host deletes its password first for the same reason.
//!
//! Changes run one at a time. Network calls run outside that lock, and the
//! write that follows one checks that the host is still the one that was
//! reached.

use std::sync::Arc;

use mannie_plugin_sdk::contracts::{Clock, Secret, UtcMillis};
use mannie_plugin_sdk::ipc::PluginError as _;
use mannie_plugin_sdk::services::secrets::{SecretsApi, SecretsError};
use mannie_plugin_sdk::services::store::{ModuleStoreApi, StoreError};
use russh::keys::PublicKey;
use tokio::sync::{Mutex, watch};

use crate::client::{self, ConnectError, SeenKey};
use crate::commands::{
    AddHost, HostKeyScan, HostKeyView, HostView, KeyComparison, SshError, UpdateHost,
};
use crate::rules::stored_address;
use crate::sessions::{Lease, SessionKey, Sessions};
use crate::store::{HostFields, HostRecord, HostsStore, NAME_TAKEN, StoredKey};

/// The credential store's entry of a host's password.
pub(crate) fn password_entry(host_id: &str) -> String {
    format!("host/{host_id}")
}

/// The registered hosts. Cloning is cheap; clones share the hosts.
#[derive(Debug, Clone)]
pub(crate) struct Hosts {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    store: HostsStore,
    secrets: Arc<dyn SecretsApi>,
    clock: Arc<dyn Clock>,
    sessions: Sessions,
    /// The hosts as the store keeps them, by name.
    records: watch::Sender<Vec<HostRecord>>,
    /// Held by each change, so changes run one at a time.
    changes: Mutex<()>,
}

/// What `connect` gives a call: a session, and the password, which the
/// call's output must not show.
#[derive(Debug)]
pub(crate) struct Connected {
    pub(crate) lease: Lease,
    pub(crate) password: Secret,
}

impl Hosts {
    /// The hosts that the store keeps.
    pub(crate) async fn load(
        store: Arc<dyn ModuleStoreApi>,
        secrets: Arc<dyn SecretsApi>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, StoreError> {
        let store = HostsStore::new(store);
        let records = store.hosts().await?;
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                secrets,
                clock,
                sessions: Sessions::default(),
                records: watch::Sender::new(records),
                changes: Mutex::new(()),
            }),
        })
    }

    /// The hosts from now on, until the plugin stops.
    pub(crate) fn follow(&self) -> watch::Receiver<Vec<HostRecord>> {
        self.inner.records.subscribe()
    }

    /// The hosts as they are, by name.
    pub(crate) fn records(&self) -> Vec<HostRecord> {
        self.inner.records.borrow().clone()
    }

    /// The host with this ID.
    pub(crate) fn by_id(&self, host_id: &str) -> Option<HostRecord> {
        self.inner
            .records
            .borrow()
            .iter()
            .find(|host| host.id == host_id)
            .cloned()
    }

    /// The host with this name, without regard to case.
    pub(crate) fn by_name(&self, name: &str) -> Option<HostRecord> {
        self.inner
            .records
            .borrow()
            .iter()
            .find(|host| host.name.eq_ignore_ascii_case(name))
            .cloned()
    }

    /// The open sessions.
    pub(crate) fn sessions(&self) -> &Sessions {
        &self.inner.sessions
    }

    fn now(&self) -> UtcMillis {
        self.inner.clock.now()
    }

    /// Reads the hosts again and hands them to everyone who follows them.
    async fn reload(&self) -> Result<(), SshError> {
        let records = self
            .inner
            .store
            .hosts()
            .await
            .map_err(|error| storage_failed(&error))?;
        self.inner.records.send_replace(records);
        Ok(())
    }

    /// The view of a host after a change.
    fn view(&self, host_id: &str) -> Result<HostView, SshError> {
        self.by_id(host_id)
            .map(|host| host.view())
            .ok_or(SshError::HostNotFound)
    }

    /// Registers a host and keeps its password.
    pub(crate) async fn add(&self, input: AddHost) -> Result<HostView, SshError> {
        let _change = self.inner.changes.lock().await;
        let id = uuid::Uuid::now_v7().to_string();
        let fields = fields(
            input.name,
            input.note,
            &input.address,
            input.port,
            input.username,
        );
        self.inner
            .store
            .insert(&id, &fields, self.now())
            .await
            .map_err(name_or_storage)?;
        let saved = self
            .inner
            .secrets
            .save(&password_entry(&id), input.password)
            .await;
        if let Err(error) = saved {
            // No host stays registered without the password that the owner
            // gave, so adding it again works.
            if let Err(error) = self.inner.store.delete(&id).await {
                tracing::warn!(host = %id, error.code = error.code(), "could not forget a host whose password was not kept");
            }
            self.reload().await?;
            return Err(secrets_failed(error));
        }
        self.inner
            .store
            .set_password_set(&id, true, self.now())
            .await
            .map_err(|error| storage_failed(&error))?;
        self.reload().await?;
        tracing::info!(host = %id, "the owner registered an SSH host");
        self.view(&id)
    }

    /// Changes a host, and its password when the input has one. Another
    /// address or port lets go of the host's keys, since they belong to the
    /// server at the old one.
    pub(crate) async fn update(&self, input: UpdateHost) -> Result<HostView, SshError> {
        let _change = self.inner.changes.lock().await;
        let current = self.by_id(&input.host_id).ok_or(SshError::HostNotFound)?;
        let fields = fields(
            input.name,
            input.note,
            &input.address,
            input.port,
            input.username,
        );
        let moved = current.address != fields.address || current.port != fields.port;
        self.inner
            .store
            .update(&current.id, &fields, moved, self.now())
            .await
            .map_err(|error| match error {
                StoreError::Expectation { .. } => SshError::HostNotFound,
                other => name_or_storage(other),
            })?;
        if let Some(password) = input.password {
            self.inner
                .secrets
                .save(&password_entry(&current.id), password)
                .await
                .map_err(secrets_failed)?;
            self.inner
                .store
                .set_password_set(&current.id, true, self.now())
                .await
                .map_err(|error| storage_failed(&error))?;
        }
        // The endpoint, the key or the password of the open session may have
        // changed.
        self.inner.sessions.forget(&current.id);
        self.reload().await?;
        tracing::info!(host = %current.id, moved, "the owner changed an SSH host");
        self.view(&current.id)
    }

    /// Deletes a host's password, then forgets the host.
    pub(crate) async fn remove(&self, host_id: &str) -> Result<(), SshError> {
        let _change = self.inner.changes.lock().await;
        let current = self.by_id(host_id).ok_or(SshError::HostNotFound)?;
        self.inner
            .secrets
            .delete(&password_entry(&current.id))
            .await
            .map_err(secrets_failed)?;
        self.inner
            .store
            .delete(&current.id)
            .await
            .map_err(|error| storage_failed(&error))?;
        self.inner.sessions.forget(&current.id);
        self.reload().await?;
        tracing::info!(host = %current.id, "the owner removed an SSH host");
        Ok(())
    }

    /// Reads the key that a host shows, records it and compares it with the
    /// pinned one. No password is sent.
    pub(crate) async fn scan(&self, host_id: &str) -> Result<HostKeyScan, SshError> {
        let current = self.by_id(host_id).ok_or(SshError::HostNotFound)?;
        let seen = client::scan(&current.address, current.port)
            .await
            .map_err(connect_failed)?;
        let recorded = self.record_seen(&current, &seen).await?;
        let comparison = match &current.key {
            None => KeyComparison::NotPinned,
            Some(key) if key.fingerprint == seen.fingerprint => KeyComparison::Same,
            Some(_other) => KeyComparison::Changed,
        };
        tracing::info!(host = %current.id, ?comparison, "the owner scanned the key of an SSH host");
        Ok(HostKeyScan {
            key: HostKeyView {
                algorithm: recorded.algorithm,
                fingerprint: recorded.fingerprint,
                at: UtcMillis::from_millis(recorded.at),
            },
            comparison,
        })
    }

    /// Records a key that `host` showed, when the host still has the address
    /// and port that were reached.
    async fn record_seen(&self, host: &HostRecord, seen: &SeenKey) -> Result<StoredKey, SshError> {
        let _change = self.inner.changes.lock().await;
        let current = self.by_id(&host.id).ok_or(SshError::HostNotFound)?;
        let is_same_server = current.address == host.address && current.port == host.port;
        if !is_same_server {
            return Err(SshError::ScanOutdated);
        }
        let stored = StoredKey {
            algorithm: seen.algorithm.clone(),
            openssh: seen.openssh.clone(),
            fingerprint: seen.fingerprint.clone(),
            at: self.now().as_millis(),
        };
        self.inner
            .store
            .record_seen(&host.id, &stored)
            .await
            .map_err(|error| match error {
                StoreError::Expectation { .. } => SshError::HostNotFound,
                other => storage_failed(&other),
            })?;
        self.reload().await?;
        Ok(stored)
    }

    /// Pins the key that the last scan of the host saw, when its
    /// fingerprint is the one that the owner checked.
    pub(crate) async fn trust(
        &self,
        host_id: &str,
        fingerprint: &str,
    ) -> Result<HostView, SshError> {
        let _change = self.inner.changes.lock().await;
        let current = self.by_id(host_id).ok_or(SshError::HostNotFound)?;
        let trusted = self
            .inner
            .store
            .trust_seen(&current.id, fingerprint, self.now())
            .await
            .map_err(|error| storage_failed(&error))?;
        if !trusted {
            return Err(SshError::ScanOutdated);
        }
        self.inner.sessions.forget(&current.id);
        self.reload().await?;
        tracing::info!(host = %current.id, "the owner trusted the key of an SSH host");
        self.view(&current.id)
    }

    /// Logs in to a host and runs nothing.
    pub(crate) async fn test(&self, host_id: &str) -> Result<(), SshError> {
        let current = self.by_id(host_id).ok_or(SshError::HostNotFound)?;
        let pin = pin_of(&current)?;
        let password = self.password(&current.id).await?;
        let mut session = self.handshake(&current, pin).await?;
        let login = client::log_in(&mut session, &current.username, &password).await;
        client::close(&session).await;
        login.map_err(connect_failed)?;
        tracing::info!(host = %current.id, "a test login to an SSH host succeeded");
        Ok(())
    }

    /// The host's password.
    pub(crate) async fn password(&self, host_id: &str) -> Result<Secret, SshError> {
        self.inner
            .secrets
            .load(&password_entry(host_id))
            .await
            .map_err(secrets_failed)?
            .ok_or(SshError::PasswordMissing)
    }

    /// A session with `host`: the open one when it still serves the host,
    /// else a new one, logged in with the host's password. The password
    /// comes along, so the call can keep it out of its output.
    pub(crate) async fn connect(&self, host: &HostRecord) -> Result<Connected, SshError> {
        let pin = pin_of(host)?;
        let password = self.password(&host.id).await?;
        let key = SessionKey {
            endpoint: host.endpoint(),
            fingerprint: host
                .key
                .as_ref()
                .map(|key| key.fingerprint.clone())
                .unwrap_or_default(),
        };
        if let Some(lease) = self.inner.sessions.lease(&host.id, &key) {
            return Ok(Connected { lease, password });
        }
        let mut session = self.handshake(host, pin).await?;
        if let Err(error) = client::log_in(&mut session, &host.username, &password).await {
            client::close(&session).await;
            return Err(connect_failed(error));
        }
        let lease = self.inner.sessions.keep(&host.id, key, session);
        Ok(Connected { lease, password })
    }

    /// A new session with `host` before login. A host that shows another
    /// key than the pinned one is recorded as changed.
    async fn handshake(
        &self,
        host: &HostRecord,
        pin: PublicKey,
    ) -> Result<client::Session, SshError> {
        match client::connect(&host.address, host.port, pin).await {
            Ok(session) => Ok(session),
            Err(ConnectError::KeyChanged(seen)) => {
                tracing::warn!(host = %host.id, "an SSH host showed another key than the pinned one; nothing was sent");
                // The changed key is the failure either way; recording it
                // only lets the settings show it.
                if let Err(error) = self.record_seen(host, &seen).await {
                    tracing::warn!(host = %host.id, error.code = error.code(), "could not record the key that an SSH host showed");
                }
                Err(SshError::HostKeyChanged)
            }
            Err(error) => Err(connect_failed(error)),
        }
    }

    /// Closes every session, as the plugin stops.
    pub(crate) fn close_sessions(&self) {
        self.inner.sessions.close_all();
    }
}

/// The owner's fields, with the address as the plugin stores it: in lower
/// case, and an IPv6 address without brackets. The host checked the input
/// with the same rules before the handler ran.
fn fields(name: String, note: String, address: &str, port: u16, username: String) -> HostFields {
    HostFields {
        name,
        note,
        address: stored_address(address).unwrap_or_else(|| address.to_owned()),
        port,
        username,
    }
}

/// The pinned key of a host.
fn pin_of(host: &HostRecord) -> Result<PublicKey, SshError> {
    let key = host.key.as_ref().ok_or(SshError::HostKeyNotTrusted)?;
    if host.changed_key().is_some() {
        return Err(SshError::HostKeyChanged);
    }
    client::pinned_key(&key.openssh).ok_or(SshError::HostKeyNotTrusted)
}

/// What a failure to reach or log in to a host means for the owner.
pub(crate) fn connect_failed(error: ConnectError) -> SshError {
    match error {
        ConnectError::Unreachable => SshError::Unreachable,
        ConnectError::TimedOut => SshError::TimedOut,
        ConnectError::Handshake => SshError::HandshakeFailed,
        ConnectError::KeyUnsupported => SshError::HostKeyUnsupported,
        ConnectError::KeyChanged(_seen) => SshError::HostKeyChanged,
        ConnectError::AuthFailed => SshError::AuthFailed,
        ConnectError::AuthUnsupported => SshError::AuthMethodUnsupported,
    }
}

fn storage_failed(error: &StoreError) -> SshError {
    tracing::warn!(error.code = error.code(), "the SSH plugin's store failed");
    SshError::StorageFailed
}

fn name_or_storage(error: StoreError) -> SshError {
    match error {
        StoreError::Constraint { code: NAME_TAKEN } => SshError::NameTaken,
        other => storage_failed(&other),
    }
}

fn secrets_failed(error: SecretsError) -> SshError {
    tracing::warn!(%error, "the credential store failed for the SSH plugin");
    SshError::CredentialStoreUnavailable
}
