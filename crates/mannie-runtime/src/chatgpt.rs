//! The ChatGPT account (ADR-0062): signing in with ChatGPT, keeping the
//! account's tokens in the vault, refreshing them one at a time, and giving
//! each model call of a profile on the ChatGPT plan a token that is valid at
//! that moment.
//!
//! The tokens live in the vault as one bundle, split into parts of at most
//! 1,000 UTF-16 units, since Windows holds 1,280 in one entry (decision 6).
//! New parts are saved before the store's row names them, and the old ones
//! are deleted after, so a crash in between keeps a bundle that the row
//! names. Each token is kept out of the logs while the process holds it.
//!
//! OpenAI replaces the refresh token on every refresh, so a refresh runs in a
//! task of its own that holds the account's gate until the new tokens are
//! stored: a run that stops drops only its wait, never a refresh that the
//! server already rotated (decision 7).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use mannie_provider::chatgpt::{
    Authorization, AuthorizationRequest, Callback, CallbackError, CallbackListener,
    ChatGptEndpoints, IdClaims, IdTokenError, ListenError, PLAN_SCOPE, TokenError, TokenSet,
    authorization, exchange_code, new_host_id, refresh_tokens, revoke_refresh_token,
    verify_id_token,
};
use mannie_provider::{ProviderError, Url, list_plan_models};
use mannie_store::{ChatGptSignInRecord, ProfileStoreError, StoredChatGptAccount};
use mannie_types::clock::UtcMillis;
use mannie_types::provider::{
    ChatGptAccountState, ChatGptAccountView, ChatGptCallbackPage, ChatGptError, ChatGptModel,
    ChatGptSignIn, CredentialStoreProblem, SignOutReport, StartChatGptSignIn,
};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::sync::watch;
use zeroize::Zeroizing;

use crate::compaction::to_hex;
use crate::profiles::{Protection, VaultError, in_vault};
use crate::{Runtime, lock};

/// How long before its expiry an access token is refreshed.
const REFRESH_MARGIN: Duration = Duration::from_mins(5);

/// The shortest lifetime of an access token that Mannie believes.
const MIN_LIFETIME: Duration = Duration::from_mins(1);

/// The longest lifetime of an access token that Mannie believes.
const MAX_LIFETIME: Duration = Duration::from_hours(24);

/// The lifetime of an access token whose answer named none: OpenAI's hour.
const DEFAULT_LIFETIME: Duration = Duration::from_hours(1);

/// The most UTF-16 units of one part in the vault.
const PART_UNITS: usize = 1_000;

/// The most parts of a bundle, as the store's row allows.
const MAX_PARTS: usize = 32;

/// How long a sign-in waits for the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_mins(5);

/// How long quitting waits for a refresh in flight.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

/// How many times signing out asks OpenAI to revoke the session.
const REVOKE_ATTEMPTS: u32 = 3;

/// How long signing out waits before it asks again.
const REVOKE_DELAY: Duration = Duration::from_secs(1);

/// What tells ChatGPT accounts apart in the bindings of provider-private
/// data (ADR-0062, decision 9): the first 16 hexadecimal digits of the
/// SHA-256 of the issued client and the account's subject. A refresh never
/// changes it, and the same account signing in again keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccountKey(String);

impl AccountKey {
    fn of(client_id: &str, subject: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(client_id.as_bytes());
        hasher.update([0]);
        hasher.update(subject.as_bytes());
        let digest = hasher.finalize();
        Self(to_hex(digest.get(..8).unwrap_or_default()))
    }

    fn of_account(account: &StoredChatGptAccount) -> Option<Self> {
        match (&account.client_id, &account.subject) {
            (Some(client_id), Some(subject)) => Some(Self::of(client_id, subject)),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A token for one model call, with the serial of the session it came from,
/// so that a refusal of an outdated token sends no second refresh.
#[derive(Debug, Clone)]
pub(crate) struct IssuedToken {
    pub(crate) token: Secret,
    pub(crate) serial: u64,
}

/// The tokens that the process holds.
#[derive(Debug)]
struct Session {
    key: AccountKey,
    /// The generation of the parts that hold these tokens, or that the
    /// tokens replace while the vault does not hold them yet.
    generation: u32,
    access: Secret,
    refresh: Secret,
    expires_at: UtcMillis,
    serial: u64,
    /// Whether a call was refused with this access token.
    rejected: bool,
    /// Whether the vault holds these tokens.
    stored: bool,
    /// Keep the tokens out of the logs.
    protections: Vec<Protection>,
    /// Keep the tokens that these replaced out of the logs until the next
    /// replacement, since a call made with them may still fail.
    previous: Vec<Protection>,
}

/// A sign-in that waits for the browser.
#[derive(Debug)]
struct PendingSignIn {
    id: u64,
    url: Url,
    task: tokio::task::AbortHandle,
}

/// The state of the ChatGPT account in a runtime.
#[derive(Debug)]
pub(crate) struct ChatGptState {
    session: RwLock<Option<Session>>,
    /// One load, refresh, sign-in or sign-out at a time.
    gate: Arc<tokio::sync::Mutex<()>>,
    sign_in: Mutex<Option<PendingSignIn>>,
    view: watch::Sender<ChatGptAccountView>,
    endpoints: RwLock<ChatGptEndpoints>,
    serials: AtomicU64,
}

impl ChatGptState {
    pub(crate) fn new(endpoints: ChatGptEndpoints) -> Self {
        Self {
            session: RwLock::new(None),
            gate: Arc::new(tokio::sync::Mutex::new(())),
            sign_in: Mutex::new(None),
            view: watch::Sender::new(ChatGptAccountView {
                state: ChatGptAccountState::SignedOut,
                email: None,
                known: false,
                notice_seen: false,
                sign_in: ChatGptSignIn::Idle,
            }),
            endpoints: RwLock::new(endpoints),
            serials: AtomicU64::new(1),
        }
    }

    fn next_serial(&self) -> u64 {
        self.serials.fetch_add(1, Ordering::Relaxed)
    }
}

/// The bundle of tokens as the vault keeps it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleText<'a> {
    v: u32,
    access: &'a str,
    refresh: &'a str,
    expires_at: i64,
}

/// The bundle of tokens as it is read back.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredBundle {
    v: u32,
    access: Secret,
    refresh: Secret,
    expires_at: i64,
}

/// The vault's entry of one part of a bundle. `provider` is the core's
/// prefix, which no plugin can name.
fn part_reference(generation: u32, part: usize) -> String {
    format!("provider/chatgpt/{generation}-{part}")
}

/// A bundle's text cut into parts of at most [`PART_UNITS`] UTF-16 units.
fn split_parts(text: &str) -> Vec<Secret> {
    let mut parts = Vec::new();
    let mut part = Zeroizing::new(String::new());
    let mut units = 0;
    for character in text.chars() {
        let width = character.len_utf16();
        if units + width > PART_UNITS {
            parts.push(Secret::new(std::mem::take(&mut *part)));
            units = 0;
        }
        part.push(character);
        units += width;
    }
    if !part.is_empty() {
        parts.push(Secret::new(std::mem::take(&mut *part)));
    }
    parts
}

/// When an access token that the answer says lasts `expires_in` seconds
/// expires.
fn expiry(now: UtcMillis, expires_in: Option<u64>) -> UtcMillis {
    let lifetime = expires_in
        .map_or(DEFAULT_LIFETIME, Duration::from_secs)
        .clamp(MIN_LIFETIME, MAX_LIFETIME);
    now.saturating_add(lifetime)
}

fn store_error(error: &ProfileStoreError) -> ChatGptError {
    tracing::error!(%error, error.code = error.code(), "the ChatGPT account could not be read or saved");
    ChatGptError::StorageFailed
}

fn vault_error(error: VaultError) -> ChatGptError {
    let reason = match error {
        VaultError::Unavailable(problem) => problem,
        VaultError::Missing => CredentialStoreProblem::Other,
    };
    ChatGptError::CredentialStoreUnavailable { reason }
}

impl Runtime {
    /// The account as the UI shows it, and every change of it.
    pub fn subscribe_chatgpt_account(&self) -> watch::Receiver<ChatGptAccountView> {
        self.inner.chatgpt.view.subscribe()
    }

    /// The account as the UI shows it now.
    pub fn chatgpt_account_view(&self) -> ChatGptAccountView {
        self.inner.chatgpt.view.borrow().clone()
    }

    /// Where the sign-in and the plan's calls go.
    pub(crate) fn chatgpt_endpoints(&self) -> ChatGptEndpoints {
        self.inner
            .chatgpt
            .endpoints
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Reads the account's row into the view, when the runtime starts.
    pub(crate) async fn load_chatgpt_view(&self) {
        match self.inner.store.chatgpt_account().await {
            Ok(account) => self.publish_chatgpt(account.as_ref()),
            Err(error) => {
                store_error(&error);
            }
        }
    }

    /// Shows the account's row in the view.
    fn publish_chatgpt(&self, account: Option<&StoredChatGptAccount>) {
        self.inner.chatgpt.view.send_modify(|view| {
            view.state = account.map_or(ChatGptAccountState::SignedOut, |account| account.state);
            view.email = account.and_then(|account| account.email.clone());
            view.known = account.is_some_and(|account| account.client_id.is_some());
            view.notice_seen = account.is_some_and(|account| account.notice_seen);
        });
    }

    fn set_sign_in_step(&self, step: ChatGptSignIn) {
        self.inner
            .chatgpt
            .view
            .send_modify(|view| view.sign_in = step);
    }

    fn write_session(&self) -> std::sync::RwLockWriteGuard<'_, Option<Session>> {
        self.inner
            .chatgpt
            .session
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn read_session(&self) -> std::sync::RwLockReadGuard<'_, Option<Session>> {
        self.inner
            .chatgpt
            .session
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Keeps a token out of the logs while the session holds it.
    fn protect(&self, secret: &Secret) -> Option<Protection> {
        self.inner.vault.protect(secret)
    }

    /// Puts tokens in the session, keeping the protections of the tokens
    /// they replace until the next replacement.
    fn install_session(&self, mut session: Session) {
        let mut current = self.write_session();
        if let Some(old) = current.take() {
            session.previous = old.protections;
        }
        *current = Some(session);
    }

    /// A session of new tokens.
    fn new_session(
        &self,
        key: AccountKey,
        generation: u32,
        access: Secret,
        refresh: Secret,
        expires_at: UtcMillis,
        stored: bool,
    ) -> Session {
        let protections = [&access, &refresh]
            .into_iter()
            .filter_map(|secret| self.protect(secret))
            .collect();
        Session {
            key,
            generation,
            access,
            refresh,
            expires_at,
            serial: self.inner.chatgpt.next_serial(),
            rejected: false,
            stored,
            protections,
            previous: Vec::new(),
        }
    }

    // The sign-in.

    /// Starts a sign-in: listens for the browser's return and returns the
    /// URL of OpenAI's sign-in page, which the caller opens. The sign-in
    /// ends in the background; the account's view shows how.
    pub async fn begin_chatgpt_sign_in(
        &self,
        input: StartChatGptSignIn,
    ) -> Result<Url, ChatGptError> {
        input.page.check()?;
        if lock(&self.inner.chatgpt.sign_in).is_some() {
            return Err(ChatGptError::AlreadySigningIn);
        }
        let host_id = new_host_id().map_err(|error| {
            tracing::error!(%error, "could not make a host ID for the ChatGPT sign-in");
            ChatGptError::StorageFailed
        })?;
        let account = self
            .inner
            .store
            .ensure_chatgpt_account(host_id)
            .await
            .map_err(|error| store_error(&error))?;
        let endpoints = self.chatgpt_endpoints();
        // A first sign-in, or the owner's choice of another account,
        // registers Mannie anew; a later one reuses the issued client.
        let register = input.another_account || account.client_id.is_none();
        let port = if register {
            endpoints.registration_port
        } else {
            0
        };
        let listener = CallbackListener::bind(port)
            .await
            .map_err(|error| match error {
                ListenError::PortUnavailable { port } => ChatGptError::PortInUse { port },
                ListenError::Failed(error) => {
                    tracing::warn!(%error, "the listener of a ChatGPT sign-in failed");
                    ChatGptError::Network
                }
            })?;
        let saved_client = if register {
            None
        } else {
            account.client_id.clone()
        };
        let login_hint = if register {
            None
        } else {
            account.email.as_deref()
        };
        let authorization = authorization(&AuthorizationRequest {
            endpoints: &endpoints,
            host_id: &account.host_id,
            port: listener.port(),
            client_id: saved_client.as_deref(),
            login_hint,
        })
        .map_err(|error| {
            tracing::error!(%error, "could not make the values of a ChatGPT sign-in");
            ChatGptError::StorageFailed
        })?;
        let url = authorization.url.clone();
        let id = self.inner.chatgpt.next_serial();
        // The pending sign-in is recorded while its task cannot end yet, so
        // the task always finds it to remove.
        let mut pending = lock(&self.inner.chatgpt.sign_in);
        if pending.is_some() {
            return Err(ChatGptError::AlreadySigningIn);
        }
        self.set_sign_in_step(ChatGptSignIn::WaitingForBrowser);
        let client = if register {
            SignInClient::Register
        } else {
            SignInClient::Saved(saved_client)
        };
        let task = tokio::spawn(self.clone().run_chatgpt_sign_in(
            id,
            listener,
            authorization,
            client,
            input.page,
        ));
        *pending = Some(PendingSignIn {
            id,
            url: url.clone(),
            task: task.abort_handle(),
        });
        drop(pending);
        tracing::info!(register, "started a ChatGPT sign-in");
        Ok(url)
    }

    /// The URL of the sign-in that waits for the browser, to open again.
    pub fn chatgpt_sign_in_page(&self) -> Result<Url, ChatGptError> {
        lock(&self.inner.chatgpt.sign_in)
            .as_ref()
            .map(|pending| pending.url.clone())
            .ok_or(ChatGptError::NotSigningIn)
    }

    /// Cancels the sign-in that waits for the browser.
    pub fn cancel_chatgpt_sign_in(&self) -> Result<(), ChatGptError> {
        let pending = lock(&self.inner.chatgpt.sign_in)
            .take()
            .ok_or(ChatGptError::NotSigningIn)?;
        pending.task.abort();
        self.set_sign_in_step(ChatGptSignIn::Idle);
        tracing::info!("canceled a ChatGPT sign-in");
        Ok(())
    }

    /// The task of a sign-in: waits for the browser, redeems the code,
    /// checks the account and stores the tokens.
    async fn run_chatgpt_sign_in(
        self,
        id: u64,
        listener: CallbackListener,
        authorization: Authorization,
        client: SignInClient,
        page: ChatGptCallbackPage,
    ) {
        let result = self
            .finish_chatgpt_sign_in(listener, &authorization, client, &page)
            .await;
        {
            let mut pending = lock(&self.inner.chatgpt.sign_in);
            if pending.as_ref().is_some_and(|pending| pending.id == id) {
                *pending = None;
            }
        }
        match result {
            Ok(()) => {
                tracing::info!("signed in with ChatGPT");
                self.set_sign_in_step(ChatGptSignIn::Idle);
            }
            Err(error) => {
                tracing::warn!(error.code = error.code(), "a ChatGPT sign-in failed");
                self.set_sign_in_step(ChatGptSignIn::Failed { error });
            }
        }
    }

    async fn finish_chatgpt_sign_in(
        &self,
        listener: CallbackListener,
        authorization: &Authorization,
        client: SignInClient,
        page: &ChatGptCallbackPage,
    ) -> Result<(), ChatGptError> {
        let callback = listener
            .wait(&authorization.state, page, SIGN_IN_TIMEOUT)
            .await
            .map_err(|error| match error {
                CallbackError::TimedOut => ChatGptError::TimedOut,
                CallbackError::Refused => ChatGptError::AccessDenied,
                CallbackError::Failed => ChatGptError::Network,
            })?;
        self.set_sign_in_step(ChatGptSignIn::Finishing);
        let outcome = self.redeem(&callback, authorization, client).await;
        callback.answer(page, outcome.is_ok()).await;
        outcome
    }

    /// Redeems the browser's code and keeps the account, when OpenAI
    /// granted the plan's use.
    async fn redeem(
        &self,
        callback: &Callback,
        authorization: &Authorization,
        client: SignInClient,
    ) -> Result<(), ChatGptError> {
        let endpoints = self.chatgpt_endpoints();
        // RFC 9207: a return that names another issuer comes from elsewhere.
        if let Some(issuer) = &callback.issuer
            && issuer.trim_end_matches('/') != endpoints.issuer
        {
            return Err(ChatGptError::SignInRefused);
        }
        let client_id = match client {
            SignInClient::Register => callback
                .client_id
                .clone()
                .ok_or(ChatGptError::RegistrationIncomplete)?,
            SignInClient::Saved(saved) => saved.ok_or(ChatGptError::RegistrationIncomplete)?,
        };
        let tokens = exchange_code(
            &self.inner.client,
            &endpoints,
            &client_id,
            &callback.code,
            authorization,
        )
        .await
        .map_err(|error| match error {
            TokenError::Transient { .. } => ChatGptError::Network,
            TokenError::Terminal { .. }
            | TokenError::InvalidClient
            | TokenError::Refused { .. }
            | TokenError::InvalidResponse => ChatGptError::SignInRefused,
        })?;
        // The tokens stay out of the logs from the moment they arrive.
        let _protections: Vec<Protection> = [
            Some(&tokens.access_token),
            tokens.refresh_token.as_ref(),
            tokens.id_token.as_ref(),
        ]
        .into_iter()
        .flatten()
        .filter_map(|secret| self.protect(secret))
        .collect();
        let id_token = tokens
            .id_token
            .as_ref()
            .ok_or(ChatGptError::IdTokenInvalid)?;
        let claims = verify_id_token(
            &self.inner.client,
            &endpoints,
            id_token,
            &client_id,
            &authorization.nonce,
        )
        .await
        .map_err(|error| match error {
            IdTokenError::KeysUnavailable { .. } => ChatGptError::Network,
            other => {
                tracing::warn!(error = %other, "the ID token of a ChatGPT sign-in failed its check");
                ChatGptError::IdTokenInvalid
            }
        })?;
        if !tokens.grants(PLAN_SCOPE) {
            // Mannie uses a sign-in only for the plan; one without the plan's
            // permission ends at OpenAI too.
            if let Some(refresh) = &tokens.refresh_token
                && let Err(error) =
                    revoke_refresh_token(&self.inner.client, &endpoints, &client_id, refresh).await
            {
                tracing::warn!(%error, "could not revoke a ChatGPT sign-in without the plan's permission");
            }
            return Err(ChatGptError::PlanUsageNotGranted);
        }
        self.keep_chatgpt_sign_in(client_id, claims, tokens).await
    }

    /// Stores the tokens of a sign-in and the account they belong to.
    async fn keep_chatgpt_sign_in(
        &self,
        client_id: String,
        claims: IdClaims,
        tokens: TokenSet,
    ) -> Result<(), ChatGptError> {
        let refresh = tokens.refresh_token.ok_or(ChatGptError::SignInRefused)?;
        let _gate = Arc::clone(&self.inner.chatgpt.gate).lock_owned().await;
        let account = self
            .inner
            .store
            .chatgpt_account()
            .await
            .map_err(|error| store_error(&error))?
            .ok_or(ChatGptError::StorageFailed)?;
        let generation = account.token_generation.saturating_add(1);
        let key = AccountKey::of(&client_id, &claims.subject);
        let now = self.inner.clock.now();
        let session = self.new_session(
            key,
            generation,
            tokens.access_token,
            refresh,
            expiry(now, tokens.expires_in),
            true,
        );
        let parts = self
            .save_bundle(generation, &session)
            .await
            .map_err(vault_error)?;
        let recorded = self
            .inner
            .store
            .record_chatgpt_sign_in(ChatGptSignInRecord {
                replaces_generation: account.token_generation,
                client_id,
                subject: claims.subject,
                email: claims.email,
                token_generation: generation,
                token_parts: parts,
            })
            .await;
        let after = match recorded {
            Ok(Some(after)) => after,
            Ok(None) => {
                self.delete_parts(generation, parts).await;
                tracing::error!("the ChatGPT account changed while a sign-in stored its tokens");
                return Err(ChatGptError::StorageFailed);
            }
            Err(error) => {
                self.delete_parts(generation, parts).await;
                return Err(store_error(&error));
            }
        };
        self.delete_parts(account.token_generation, account.token_parts)
            .await;
        self.install_session(session);
        self.publish_chatgpt(Some(&after));
        Ok(())
    }

    // Signing out, the catalog and the notice.

    /// Signs out: revokes the session at OpenAI, then forgets the tokens.
    /// When OpenAI does not confirm the revocation, the tokens are forgotten
    /// anyway, and the report says so.
    pub async fn sign_out_chatgpt(&self) -> Result<SignOutReport, ChatGptError> {
        let pending = lock(&self.inner.chatgpt.sign_in).take();
        if let Some(pending) = pending {
            pending.task.abort();
            self.set_sign_in_step(ChatGptSignIn::Idle);
        }
        let _gate = Arc::clone(&self.inner.chatgpt.gate).lock_owned().await;
        let account = self
            .inner
            .store
            .chatgpt_account()
            .await
            .map_err(|error| store_error(&error))?;
        let mut revoked = true;
        if let Some(account) = account
            .as_ref()
            .filter(|account| account.state == ChatGptAccountState::SignedIn)
            && let Some(client_id) = account.client_id.clone()
        {
            revoked = match self.held_refresh_token(account).await {
                Some(refresh) => {
                    let _protection = self.protect(&refresh);
                    self.revoke(&client_id, &refresh).await
                }
                None => false,
            };
        }
        let before = self
            .inner
            .store
            .record_chatgpt_sign_out()
            .await
            .map_err(|error| store_error(&error))?;
        if let Some(before) = &before {
            self.delete_parts(before.token_generation, before.token_parts)
                .await;
        }
        self.write_session().take();
        let after = self
            .inner
            .store
            .chatgpt_account()
            .await
            .map_err(|error| store_error(&error))?;
        self.publish_chatgpt(after.as_ref());
        tracing::info!(revoked, "signed out of ChatGPT");
        Ok(SignOutReport { revoked })
    }

    /// The refresh token of the signed-in account, from the session or the
    /// vault.
    async fn held_refresh_token(&self, account: &StoredChatGptAccount) -> Option<Secret> {
        if let Some(session) = self.read_session().as_ref() {
            return Some(session.refresh.clone());
        }
        match self.read_bundle(account).await {
            Ok(Some(bundle)) => Some(bundle.refresh),
            Ok(None) | Err(_) => None,
        }
    }

    /// Asks OpenAI to revoke a refresh token, trying again after failures
    /// that pass.
    async fn revoke(&self, client_id: &str, refresh: &Secret) -> bool {
        let endpoints = self.chatgpt_endpoints();
        for attempt in 1..=REVOKE_ATTEMPTS {
            match revoke_refresh_token(&self.inner.client, &endpoints, client_id, refresh).await {
                Ok(()) => return true,
                Err(TokenError::Transient { cause, .. }) if attempt < REVOKE_ATTEMPTS => {
                    tracing::info!(
                        cause = cause.code(),
                        attempt,
                        "revoking the ChatGPT session failed for now; trying again"
                    );
                    tokio::time::sleep(REVOKE_DELAY.saturating_mul(attempt)).await;
                }
                Err(error) => {
                    tracing::warn!(%error, "OpenAI did not confirm that it revoked the ChatGPT session");
                    return false;
                }
            }
        }
        false
    }

    /// The models that the plan offers the signed-in account, in the order
    /// of OpenAI's catalog.
    pub async fn chatgpt_models(&self) -> Result<Vec<ChatGptModel>, ChatGptError> {
        let key = self.chatgpt_key().await.map_err(|failure| match failure {
            RunFailure::ChatGptSignInNeeded => ChatGptError::SignedOut,
            failure => ChatGptError::Provider { failure },
        })?;
        let api_base = self.chatgpt_endpoints().api_base;
        let mut refused_once = false;
        loop {
            let issued =
                self.chatgpt_token(&key)
                    .await
                    .map_err(|error| ChatGptError::Provider {
                        failure: error.failure(),
                    })?;
            match list_plan_models(&self.inner.client, &api_base, &issued.token).await {
                Ok(models) => {
                    return Ok(models
                        .into_iter()
                        .map(|model| ChatGptModel {
                            display_name: model.display_name.unwrap_or_else(|| model.id.clone()),
                            slug: model.id,
                        })
                        .collect());
                }
                Err(ProviderError::Permanent(RunFailure::InvalidCredential)) if !refused_once => {
                    refused_once = true;
                    self.reject_chatgpt_token(Some(issued.serial));
                }
                Err(error) => {
                    return Err(ChatGptError::Provider {
                        failure: error.failure(),
                    });
                }
            }
        }
    }

    /// Records that the owner saw the notice that Mannie uses their plan.
    pub async fn acknowledge_chatgpt_notice(&self) -> Result<(), ChatGptError> {
        self.inner
            .store
            .record_chatgpt_notice_seen()
            .await
            .map_err(|error| store_error(&error))?;
        let account = self
            .inner
            .store
            .chatgpt_account()
            .await
            .map_err(|error| store_error(&error))?;
        self.publish_chatgpt(account.as_ref());
        Ok(())
    }

    // The tokens of model calls.

    /// The key of the signed-in account, for the setup of a run on the plan.
    pub(crate) async fn chatgpt_key(&self) -> Result<AccountKey, RunFailure> {
        match self.inner.store.chatgpt_account().await {
            Ok(Some(account)) if account.state == ChatGptAccountState::SignedIn => {
                AccountKey::of_account(&account).ok_or(RunFailure::Internal)
            }
            Ok(_) => Err(RunFailure::ChatGptSignInNeeded),
            Err(error) => {
                store_error(&error);
                Err(RunFailure::Internal)
            }
        }
    }

    /// A token for a call of the account with `key`, valid for at least the
    /// refresh margin.
    pub(crate) async fn chatgpt_token(
        &self,
        key: &AccountKey,
    ) -> Result<IssuedToken, ProviderError> {
        if let Some(issued) = self.fresh_chatgpt_token(key) {
            return issued;
        }
        let runtime = self.clone();
        let key = key.clone();
        // The task holds the gate until rotated tokens are stored; the caller
        // may stop waiting, the refresh still ends (ADR-0062, decision 7).
        tokio::spawn(async move {
            let _gate = Arc::clone(&runtime.inner.chatgpt.gate).lock_owned().await;
            if let Some(issued) = runtime.fresh_chatgpt_token(&key) {
                return issued;
            }
            runtime.load_or_refresh(&key).await
        })
        .await
        .unwrap_or(Err(ProviderError::Permanent(RunFailure::Internal)))
    }

    /// The session's token when it is valid for the refresh margin, an
    /// error when the session belongs to another account, and `None` when
    /// the token must be loaded or refreshed first.
    fn fresh_chatgpt_token(&self, key: &AccountKey) -> Option<Result<IssuedToken, ProviderError>> {
        let session = self.read_session();
        let session = session.as_ref()?;
        if session.key != *key {
            return Some(Err(ProviderError::Permanent(
                RunFailure::ChatGptAccountChanged,
            )));
        }
        let fresh_until = self.inner.clock.now().saturating_add(REFRESH_MARGIN);
        if session.rejected || session.expires_at <= fresh_until {
            return None;
        }
        Some(Ok(IssuedToken {
            token: session.access.clone(),
            serial: session.serial,
        }))
    }

    /// Marks the token of `serial` as refused, so that the next call
    /// refreshes it. A refusal of a token that was replaced since changes
    /// nothing.
    pub(crate) fn reject_chatgpt_token(&self, serial: Option<u64>) {
        let Some(serial) = serial else {
            return;
        };
        if let Some(session) = self.write_session().as_mut()
            && session.serial == serial
        {
            session.rejected = true;
        }
    }

    /// Loads the tokens that the vault holds, refreshes them when they
    /// expire soon, and returns the current token. The caller holds the
    /// gate.
    async fn load_or_refresh(&self, key: &AccountKey) -> Result<IssuedToken, ProviderError> {
        let account = match self.inner.store.chatgpt_account().await {
            Ok(account) => account,
            Err(error) => {
                store_error(&error);
                return Err(ProviderError::Permanent(RunFailure::Internal));
            }
        };
        let Some(account) =
            account.filter(|account| account.state == ChatGptAccountState::SignedIn)
        else {
            self.write_session().take();
            return Err(ProviderError::Permanent(RunFailure::ChatGptSignInNeeded));
        };
        let (Some(account_key), Some(client_id)) =
            (AccountKey::of_account(&account), account.client_id.clone())
        else {
            return Err(ProviderError::Permanent(RunFailure::Internal));
        };
        if account_key != *key {
            return Err(ProviderError::Permanent(RunFailure::ChatGptAccountChanged));
        }
        let loaded = self.read_session().as_ref().is_some_and(|session| {
            session.key == *key && session.generation == account.token_generation
        });
        if !loaded {
            let bundle = match self.read_bundle(&account).await {
                Ok(Some(bundle)) => bundle,
                Ok(None) => {
                    tracing::warn!(
                        "the ChatGPT tokens are missing from the credential store; the owner signs in again"
                    );
                    self.end_chatgpt_session(&account).await;
                    return Err(ProviderError::Permanent(RunFailure::ChatGptSignInNeeded));
                }
                Err(error) => {
                    tracing::warn!(%error, "the credential store could not read the ChatGPT tokens");
                    return Err(ProviderError::Permanent(
                        RunFailure::CredentialStoreUnavailable,
                    ));
                }
            };
            let session = self.new_session(
                key.clone(),
                account.token_generation,
                bundle.access,
                bundle.refresh,
                UtcMillis::from_millis(bundle.expires_at),
                true,
            );
            self.install_session(session);
        }
        if let Some(Ok(issued)) = self.fresh_chatgpt_token(key) {
            self.store_unsaved_session(&account).await;
            return Ok(issued);
        }
        self.refresh_session(&account, &client_id, key).await
    }

    /// Refreshes the session's tokens and stores the new ones. The caller
    /// holds the gate.
    async fn refresh_session(
        &self,
        account: &StoredChatGptAccount,
        client_id: &str,
        key: &AccountKey,
    ) -> Result<IssuedToken, ProviderError> {
        let Some(refresh) = self
            .read_session()
            .as_ref()
            .map(|session| session.refresh.clone())
        else {
            return Err(ProviderError::Permanent(RunFailure::ChatGptSignInNeeded));
        };
        let endpoints = self.chatgpt_endpoints();
        match refresh_tokens(&self.inner.client, &endpoints, client_id, &refresh).await {
            Ok(tokens) => {
                let now = self.inner.clock.now();
                // An answer without a refresh token keeps the old one.
                let refresh = tokens.refresh_token.unwrap_or(refresh);
                let session = self.new_session(
                    key.clone(),
                    account.token_generation,
                    tokens.access_token,
                    refresh,
                    expiry(now, tokens.expires_in),
                    false,
                );
                let issued = IssuedToken {
                    token: session.access.clone(),
                    serial: session.serial,
                };
                self.install_session(session);
                tracing::info!("refreshed the ChatGPT tokens");
                self.store_unsaved_session(account).await;
                Ok(issued)
            }
            Err(TokenError::Transient { cause, retry_after }) => {
                tracing::info!(
                    cause = cause.code(),
                    "refreshing the ChatGPT tokens failed for now"
                );
                Err(ProviderError::Transient { cause, retry_after })
            }
            Err(TokenError::InvalidResponse) => Err(ProviderError::Transient {
                cause: RunFailure::InvalidResponse,
                retry_after: None,
            }),
            Err(
                error @ (TokenError::Terminal { .. }
                | TokenError::InvalidClient
                | TokenError::Refused { .. }),
            ) => {
                tracing::warn!(%error, "the ChatGPT session ended; the owner signs in again");
                self.end_chatgpt_session(account).await;
                Err(ProviderError::Permanent(RunFailure::ChatGptSignInNeeded))
            }
        }
    }

    /// Saves the session's tokens when the vault does not hold them yet: new
    /// parts first, then the row, then the old parts go. A failure keeps the
    /// tokens in this process, and the next call tries again.
    async fn store_unsaved_session(&self, account: &StoredChatGptAccount) {
        let snapshot = self
            .read_session()
            .as_ref()
            .filter(|session| !session.stored)
            .map(|session| {
                (
                    session.serial,
                    session.access.clone(),
                    session.refresh.clone(),
                    session.expires_at,
                )
            });
        let Some((serial, access, refresh, expires_at)) = snapshot else {
            return;
        };
        let from = account.token_generation;
        let to = from.saturating_add(1);
        let parts = match self
            .save_parts(to, &bundle_text(&access, &refresh, expires_at))
            .await
        {
            Ok(parts) => parts,
            Err(error) => {
                tracing::warn!(%error, "could not store the refreshed ChatGPT tokens; this process keeps them");
                return;
            }
        };
        match self
            .inner
            .store
            .record_chatgpt_tokens(from, to, parts)
            .await
        {
            Ok(true) => {
                self.delete_parts(from, account.token_parts).await;
                if let Some(session) = self.write_session().as_mut()
                    && session.serial == serial
                {
                    session.generation = to;
                    session.stored = true;
                }
            }
            Ok(false) => {
                self.delete_parts(to, parts).await;
                tracing::warn!("the ChatGPT account changed while its tokens were stored");
            }
            Err(error) => {
                self.delete_parts(to, parts).await;
                store_error(&error);
            }
        }
    }

    /// Ends the session of `account`: the owner signs in again. Its parts go.
    async fn end_chatgpt_session(&self, account: &StoredChatGptAccount) {
        match self
            .inner
            .store
            .record_chatgpt_sign_in_needed(account.token_generation)
            .await
        {
            Ok(true) => {
                self.delete_parts(account.token_generation, account.token_parts)
                    .await;
            }
            Ok(false) => {}
            Err(error) => {
                store_error(&error);
            }
        }
        self.write_session().take();
        match self.inner.store.chatgpt_account().await {
            Ok(after) => self.publish_chatgpt(after.as_ref()),
            Err(error) => {
                store_error(&error);
            }
        }
    }

    // The vault.

    /// Saves the tokens of `session` as the parts of `generation`.
    async fn save_bundle(&self, generation: u32, session: &Session) -> Result<u32, VaultError> {
        self.save_parts(
            generation,
            &bundle_text(&session.access, &session.refresh, session.expires_at),
        )
        .await
    }

    /// Saves a bundle's text as the parts of `generation`, and returns how
    /// many there are. Parts saved before a failure are deleted.
    async fn save_parts(
        &self,
        generation: u32,
        text: &Zeroizing<String>,
    ) -> Result<u32, VaultError> {
        let parts = split_parts(text);
        if parts.len() > MAX_PARTS {
            tracing::error!(
                parts = parts.len(),
                "the ChatGPT tokens are longer than the store's row allows"
            );
            return Err(VaultError::Unavailable(CredentialStoreProblem::Other));
        }
        let count = u32::try_from(parts.len()).unwrap_or(u32::MAX);
        let saved = in_vault(&self.inner.vault, move |vault| {
            for (index, part) in parts.iter().enumerate() {
                if let Err(error) = vault.save_part(&part_reference(generation, index), part) {
                    for earlier in 0..index {
                        let _ignored = vault.delete(&part_reference(generation, earlier));
                    }
                    return Err(error);
                }
            }
            Ok(())
        })
        .await;
        saved.map(|()| count)
    }

    /// The bundle that the parts of `account` hold, or `None` when a part
    /// is missing or the bundle cannot be read.
    async fn read_bundle(
        &self,
        account: &StoredChatGptAccount,
    ) -> Result<Option<StoredBundle>, VaultError> {
        let generation = account.token_generation;
        let parts = usize::try_from(account.token_parts).unwrap_or(0);
        let read = in_vault(&self.inner.vault, move |vault| {
            let mut text = Zeroizing::new(String::new());
            for part in 0..parts {
                let secret = vault.read_part(&part_reference(generation, part))?;
                text.push_str(secret.expose());
            }
            Ok(text)
        })
        .await;
        let text = match read {
            Ok(text) => text,
            Err(VaultError::Missing) => return Ok(None),
            Err(error) => return Err(error),
        };
        let bundle = serde_json::from_str::<StoredBundle>(&text)
            .ok()
            .filter(|bundle| bundle.v == 1);
        if bundle.is_none() {
            tracing::warn!("the ChatGPT tokens in the credential store cannot be read");
        }
        Ok(bundle)
    }

    /// Deletes the parts of `generation`. A failure leaves them in the
    /// vault, where the next sign-in's parts do not meet them.
    async fn delete_parts(&self, generation: u32, parts: u32) {
        if parts == 0 {
            return;
        }
        let deleted = in_vault(&self.inner.vault, move |vault| {
            let mut failed = None;
            for part in 0..usize::try_from(parts).unwrap_or(0) {
                match vault.delete(&part_reference(generation, part)) {
                    Ok(()) | Err(VaultError::Missing) => {}
                    Err(error) => failed = Some(error),
                }
            }
            failed.map_or(Ok(()), Err)
        })
        .await;
        if let Err(error) = deleted {
            tracing::warn!(%error, "could not delete ChatGPT tokens that are no longer used");
        }
    }

    /// Ends the sign-in that waits and waits for a refresh in flight, when
    /// Mannie quits.
    pub(crate) async fn quit_chatgpt(&self) {
        let pending = lock(&self.inner.chatgpt.sign_in).take();
        if let Some(pending) = pending {
            pending.task.abort();
        }
        let gate = Arc::clone(&self.inner.chatgpt.gate);
        if tokio::time::timeout(SHUTDOWN_WAIT, gate.lock_owned())
            .await
            .is_err()
        {
            tracing::warn!("a refresh of the ChatGPT tokens did not end in time");
        }
    }
}

/// The text of a bundle of tokens.
fn bundle_text(access: &Secret, refresh: &Secret, expires_at: UtcMillis) -> Zeroizing<String> {
    let bundle = BundleText {
        v: 1,
        access: access.expose(),
        refresh: refresh.expose(),
        expires_at: expires_at.as_millis(),
    };
    Zeroizing::new(serde_json::to_string(&bundle).unwrap_or_default())
}

/// Which client a sign-in signs in to.
#[derive(Debug)]
enum SignInClient {
    /// A new registration, whose client the browser's return names.
    Register,
    /// The client of an earlier sign-in.
    Saved(Option<String>),
}

#[cfg(any(feature = "testing", feature = "validation"))]
impl Runtime {
    /// Points the sign-in and the plan's calls at other servers, as tests
    /// and the validation build's end-to-end tests do.
    pub fn set_chatgpt_endpoints(&self, endpoints: ChatGptEndpoints) {
        *self
            .inner
            .chatgpt
            .endpoints
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = endpoints;
    }

    /// Makes the current access token expire, so that the next call
    /// refreshes it.
    pub fn expire_chatgpt_token(&self) {
        if let Some(session) = self.write_session().as_mut() {
            session.expires_at = UtcMillis::EPOCH;
        }
    }
}

#[cfg(test)]
mod tests {
    use mannie_types::clock::UtcMillis;
    use mannie_types::secret::Secret;

    use super::{AccountKey, PART_UNITS, bundle_text, expiry, split_parts};

    #[test]
    fn a_bundle_splits_between_characters_within_the_windows_limit() {
        let access = Secret::new("a".repeat(2_500));
        let refresh = Secret::new(format!("{}é", "r".repeat(800)));
        let text = bundle_text(&access, &refresh, UtcMillis::from_millis(5));

        let parts = split_parts(&text);

        assert_eq!(parts.len(), 4);
        assert!(
            parts
                .iter()
                .all(|part| part.expose().encode_utf16().count() <= PART_UNITS)
        );
        let joined: String = parts.iter().map(Secret::expose).collect();
        assert_eq!(joined, *text);
    }

    #[test]
    fn account_keys_tell_accounts_apart_and_stay_the_same_for_one() {
        let first = AccountKey::of("oaiapp_1", "user-1");

        assert_eq!(first, AccountKey::of("oaiapp_1", "user-1"));
        assert_ne!(first, AccountKey::of("oaiapp_1", "user-2"));
        assert_ne!(first, AccountKey::of("oaiapp_2", "user-1"));
        assert_ne!(
            AccountKey::of("ab", "c"),
            AccountKey::of("a", "bc"),
            "the separator keeps the fields apart"
        );
        assert_eq!(first.as_str().len(), 16);
    }

    #[test]
    fn lifetimes_are_kept_within_bounds() {
        let now = UtcMillis::from_millis(0);

        assert_eq!(expiry(now, Some(3_600)).as_millis(), 3_600_000);
        assert_eq!(expiry(now, None).as_millis(), 3_600_000);
        assert_eq!(expiry(now, Some(1)).as_millis(), 60_000);
        assert_eq!(expiry(now, Some(10_000_000)).as_millis(), 86_400_000);
    }
}
