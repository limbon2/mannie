//! The SSH client (plugins/ssh/docs/adr/0001-speak-ssh-with-russh-and-
//! pinned-host-keys.md): russh on ring, over a TCP connection that the
//! plugin opens itself under its `[native] sockets` grant.
//!
//! The client compares the key that a host shows with the pinned one right
//! after the key exchange and before any authentication, so a host that
//! shows another key never receives the password. A scan reads the key and
//! stops there. The client accepts Ed25519 and ECDSA host keys only: russh
//! is built without RSA (ADR-0036; D35).
//!
//! Login tries the password, then keyboard-interactive login that asks for
//! exactly one hidden answer, which is how a server such as FreeBSD's, with
//! `PasswordAuthentication no`, takes a password through PAM.

use std::borrow::Cow;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use mannie_plugin_sdk::contracts::Secret;
use russh::client::{self, AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{Algorithm, EcdsaCurve, HashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{AlgorithmKind, Disconnect, MethodKind, Preferred};
use tokio::net::TcpStream;

/// How long reaching a host and the SSH handshake may take together.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a login may take.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

/// How long an open session waits for the host before it sends a
/// keepalive, and how many may go unanswered before it closes.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const KEEPALIVE_MAX: usize = 3;

/// The most rounds of keyboard-interactive login: a server may send an
/// empty round before the one that asks for the password.
const MAX_KEYBOARD_INTERACTIVE_ROUNDS: usize = 4;

/// The host keys that the client accepts, the strongest first.
const HOST_KEY_ALGORITHMS: &[Algorithm] = &[
    Algorithm::Ed25519,
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP256,
    },
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP384,
    },
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP521,
    },
];

/// A session with a host after login, on which commands open channels.
pub(crate) type Session = Handle<Client>;

/// Why the client could not reach or log in to a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConnectError {
    /// The address could not be resolved, or nothing answered at it.
    Unreachable,
    /// The host did not answer in time.
    TimedOut,
    /// The handshake failed: the host does not speak SSH as the client
    /// does, or ended the connection.
    Handshake,
    /// The host offers no host key that the client accepts.
    KeyUnsupported,
    /// The host showed another key than the pinned one, so the client
    /// stopped before authentication.
    KeyChanged(SeenKey),
    /// The host refused the user name or the password.
    AuthFailed,
    /// The host takes no password, directly or through keyboard-interactive
    /// login.
    AuthUnsupported,
}

/// A host key as the client saw it, in the forms that the plugin stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeenKey {
    /// The algorithm, such as `ssh-ed25519`.
    pub(crate) algorithm: String,
    /// The public key in OpenSSH's format, without a comment.
    pub(crate) openssh: String,
    /// The SHA-256 fingerprint, as `ssh-keygen -l` prints it.
    pub(crate) fingerprint: String,
}

impl SeenKey {
    /// The forms of `key`, or `None` for a key that OpenSSH's format cannot
    /// hold.
    pub(crate) fn of(key: &PublicKey) -> Option<Self> {
        let mut bare = key.clone();
        bare.set_comment("");
        let openssh = bare.to_openssh().ok()?;
        Some(Self {
            algorithm: key.algorithm().to_string(),
            openssh,
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        })
    }
}

/// Reads a pinned key as the plugin stored it.
pub(crate) fn pinned_key(openssh: &str) -> Option<PublicKey> {
    PublicKey::from_openssh(openssh).ok()
}

/// The client's side of a connection: the pinned key, if any, and the key
/// that the host showed, which the caller reads after the handshake.
#[derive(Debug)]
pub(crate) struct Client {
    pin: Option<PublicKey>,
    seen: Arc<Mutex<Option<PublicKey>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    /// Accepts only the pinned key. A certificate is refused, because the
    /// owner pins keys, and Mannie trusts no certificate authority.
    fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        std::future::ready(Ok(self.is_pinned(key)))
    }
}

impl Client {
    /// Records the key that the host showed, and says whether it is the
    /// pinned one.
    fn is_pinned(&self, key: &PublicKeyOrCertificate) -> bool {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return false;
        };
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(key.clone());
        self.pin
            .as_ref()
            .is_some_and(|pin| pin.key_data() == key.key_data())
    }
}

fn config() -> Arc<client::Config> {
    Arc::new(client::Config {
        preferred: Preferred {
            key: Cow::Borrowed(HOST_KEY_ALGORITHMS),
            ..Preferred::DEFAULT
        },
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        keepalive_max: KEEPALIVE_MAX,
        nodelay: true,
        ..client::Config::default()
    })
}

/// The key that the handler recorded.
fn recorded(seen: &Mutex<Option<PublicKey>>) -> Option<PublicKey> {
    seen.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Opens the TCP connection to `address` and `port` and runs the SSH
/// handshake with `client`, within [`CONNECT_TIMEOUT`].
async fn handshake(address: &str, port: u16, client: Client) -> Result<Session, ConnectError> {
    let connecting = async {
        let stream = TcpStream::connect((address, port))
            .await
            .map_err(|error| io_error(&error))?;
        client::connect_stream(config(), stream, client)
            .await
            .map_err(|error| ssh_error(&error))
    };
    tokio::time::timeout(CONNECT_TIMEOUT, connecting)
        .await
        .unwrap_or(Err(ConnectError::TimedOut))
}

/// Reads the key that a host shows, and stops after the key exchange: no
/// authentication reaches the host.
pub(crate) async fn scan(address: &str, port: u16) -> Result<SeenKey, ConnectError> {
    let seen = Arc::new(Mutex::new(None));
    let client = Client {
        pin: None,
        seen: Arc::clone(&seen),
    };
    // Without a pin, the handler refuses every key, so the handshake ends
    // with the key recorded.
    let failure = match handshake(address, port, client).await {
        Ok(session) => {
            close(&session).await;
            ConnectError::Handshake
        }
        Err(error) => error,
    };
    match recorded(&seen) {
        Some(key) => SeenKey::of(&key).ok_or(ConnectError::KeyUnsupported),
        None => Err(failure),
    }
}

/// Connects to a host whose key `pin` is pinned. A host that shows another
/// key fails with [`ConnectError::KeyChanged`] before authentication.
pub(crate) async fn connect(
    address: &str,
    port: u16,
    pin: PublicKey,
) -> Result<Session, ConnectError> {
    let seen = Arc::new(Mutex::new(None));
    let client = Client {
        pin: Some(pin.clone()),
        seen: Arc::clone(&seen),
    };
    match handshake(address, port, client).await {
        Ok(session) => Ok(session),
        Err(error) => {
            match recorded(&seen) {
                Some(key) if key.key_data() != pin.key_data() => Err(SeenKey::of(&key)
                    .map_or(ConnectError::KeyUnsupported, ConnectError::KeyChanged)),
                _other => Err(error),
            }
        }
    }
}

/// Logs in with the password, then with keyboard-interactive login that
/// asks for exactly one hidden answer. russh takes the password as a
/// `String`, so a copy lives until the login's message is sent.
pub(crate) async fn log_in(
    session: &mut Session,
    username: &str,
    password: &Secret,
) -> Result<(), ConnectError> {
    tokio::time::timeout(LOGIN_TIMEOUT, log_in_now(session, username, password))
        .await
        .unwrap_or(Err(ConnectError::TimedOut))
}

async fn log_in_now(
    session: &mut Session,
    username: &str,
    password: &Secret,
) -> Result<(), ConnectError> {
    let direct = session
        .authenticate_password(username, password.expose())
        .await
        .map_err(|error| ssh_error(&error))?;
    let AuthResult::Failure {
        remaining_methods, ..
    } = direct
    else {
        return Ok(());
    };
    if !remaining_methods.contains(&MethodKind::KeyboardInteractive) {
        let takes_passwords = remaining_methods.contains(&MethodKind::Password);
        return Err(if takes_passwords {
            ConnectError::AuthFailed
        } else {
            ConnectError::AuthUnsupported
        });
    }
    let mut response = session
        .authenticate_keyboard_interactive_start(username, None::<String>)
        .await
        .map_err(|error| ssh_error(&error))?;
    for _round in 0..MAX_KEYBOARD_INTERACTIVE_ROUNDS {
        let prompts = match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(()),
            KeyboardInteractiveAuthResponse::Failure { .. } => {
                return Err(ConnectError::AuthFailed);
            }
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => prompts,
        };
        let answers = match prompts.as_slice() {
            [] => Vec::new(),
            [prompt] if !prompt.echo => vec![password.expose().to_owned()],
            _more => return Err(ConnectError::AuthUnsupported),
        };
        response = session
            .authenticate_keyboard_interactive_respond(answers)
            .await
            .map_err(|error| ssh_error(&error))?;
    }
    Err(ConnectError::AuthUnsupported)
}

/// Ends a session, which closes its channels.
pub(crate) async fn close(session: &Session) {
    if let Err(error) = session
        .disconnect(Disconnect::ByApplication, "", "en")
        .await
    {
        tracing::debug!(%error, "an SSH session was closed already");
    }
}

/// What a failure to open the TCP connection means.
fn io_error(error: &std::io::Error) -> ConnectError {
    tracing::debug!(%error, "the TCP connection to a host failed");
    match error.kind() {
        std::io::ErrorKind::TimedOut => ConnectError::TimedOut,
        _other => ConnectError::Unreachable,
    }
}

/// What a failure of russh means.
fn ssh_error(error: &russh::Error) -> ConnectError {
    tracing::debug!(%error, "an SSH connection failed");
    match error {
        russh::Error::NoCommonAlgo {
            kind: AlgorithmKind::Key,
            ..
        } => ConnectError::KeyUnsupported,
        russh::Error::ConnectionTimeout
        | russh::Error::KeepaliveTimeout
        | russh::Error::InactivityTimeout
        | russh::Error::Elapsed(_) => ConnectError::TimedOut,
        russh::Error::IO(io) => io_error(io),
        _other => ConnectError::Handshake,
    }
}

#[cfg(test)]
mod tests {
    use russh::keys::PublicKey;

    use super::{SeenKey, pinned_key};

    /// An Ed25519 key, whose fingerprint OpenSSH's `ssh-keygen -lf` printed
    /// on 2026-10-20 as `SHA256:3BsTpRlWni0K5eIGEPA9URiaFoYsnFlC/YHer8ymt9Y`.
    const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK3vV4xRqpU3Vo4tA4mpMl9h3XFYfNIiV1vJnMmc2X7N owner@box";

    #[test]
    fn a_key_is_kept_without_its_comment_and_shown_by_its_sha256_fingerprint() {
        let key = PublicKey::from_openssh(KEY).unwrap();

        let seen = SeenKey::of(&key).unwrap();

        assert_eq!(seen.algorithm, "ssh-ed25519");
        assert_eq!(
            seen.openssh,
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK3vV4xRqpU3Vo4tA4mpMl9h3XFYfNIiV1vJnMmc2X7N"
        );
        assert_eq!(
            seen.fingerprint,
            "SHA256:3BsTpRlWni0K5eIGEPA9URiaFoYsnFlC/YHer8ymt9Y"
        );
        assert_eq!(
            pinned_key(&seen.openssh).unwrap().key_data(),
            key.key_data()
        );
        assert!(pinned_key("ssh-ed25519 not-base64").is_none());
    }
}
