//! TLS for the link (ADR-0016; docs/features/workers/pairing-and-link.md,
//! TLS): TLS 1.3 with rustls and ring, ALPN `h2`, and raw public keys
//! (RFC 7250) in both directions, so each side presents its public key and
//! never a certificate.
//!
//! The master's verifier accepts only the worker's pinned key, or any key
//! while it pairs. The worker's verifier accepts any master that signs its
//! handshake with the key it presents, and the worker's services let only
//! the pinned master past pairing. Both check the handshake's signature
//! with the presented key, so a peer proves that it holds its key.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use rustls::client::AlwaysResolvesClientRawPublicKeys;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls13_signature_with_raw_key};
use rustls::pki_types::{CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime};
use rustls::server::AlwaysResolvesServerRawPublicKeys;
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, Error, ServerConfig,
    SignatureScheme,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tonic::transport::server::Connected;

use crate::keys::{KeyPin, LinkKey};

/// The protocol that the link speaks inside TLS.
const ALPN: &[u8] = b"h2";

/// How long a handshake may take.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How many handshakes the worker handles at once, so a flood of handshakes
/// cannot exhaust it.
pub const MAX_HANDSHAKES: usize = 8;

/// How the worker's server limits handshakes.
#[derive(Debug, Clone, Copy)]
struct HandshakeLimits {
    /// How long one handshake may take.
    timeout: Duration,
    /// How many run at once.
    at_once: usize,
}

const LIMITS: HandshakeLimits = HandshakeLimits {
    timeout: HANDSHAKE_TIMEOUT,
    at_once: MAX_HANDSHAKES,
};

/// The name that the master gives TLS for a worker. The verifier checks the
/// pinned key, never a name.
const WORKER_NAME: &str = "mannie-worker";

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Checks a handshake's signature with the raw public key that the peer
/// presented.
fn verify_signature(
    message: &[u8],
    key: &CertificateDer<'_>,
    signed: &DigitallySignedStruct,
) -> Result<HandshakeSignatureValid, Error> {
    verify_tls13_signature_with_raw_key(
        message,
        &SubjectPublicKeyInfoDer::from(key.as_ref()),
        signed,
        &provider().signature_verification_algorithms,
    )
}

fn tls12_refused() -> Error {
    Error::General("the link speaks TLS 1.3 only".to_owned())
}

/// The worker's verifier of masters: any key whose holder signs the
/// handshake. Which master may use what is decided after the handshake, by
/// the pin of the key.
#[derive(Debug)]
struct MasterKeys;

impl ClientCertVerifier for MasterKeys {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _key: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _key: &CertificateDer<'_>,
        _signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(tls12_refused())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        key: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_signature(message, key, signed)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// The master's verifier of a worker: only its pinned key, or any key while
/// the master pairs with it.
#[derive(Debug)]
struct WorkerKey {
    pinned: Option<KeyPin>,
}

impl ServerCertVerifier for WorkerKey {
    fn verify_server_cert(
        &self,
        key: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        match &self.pinned {
            Some(pinned) if KeyPin::of_spki(key.as_ref()) != *pinned => Err(
                Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure),
            ),
            Some(_) | None => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _key: &CertificateDer<'_>,
        _signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(tls12_refused())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        key: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_signature(message, key, signed)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// The worker's TLS configuration: its own key, and the masters' keys
/// recorded from their handshakes.
pub(crate) fn server_config(key: &LinkKey) -> Result<Arc<ServerConfig>, Error> {
    let mut config = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(MasterKeys))
        .with_cert_resolver(Arc::new(AlwaysResolvesServerRawPublicKeys::new(
            key.certified_key(),
        )));
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(Arc::new(config))
}

/// The master's TLS configuration for one worker: the master's key, and the
/// worker's pinned key, or `None` while pairing.
pub fn client_config(key: &LinkKey, pinned: Option<KeyPin>) -> Result<Arc<ClientConfig>, Error> {
    let mut config = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(WorkerKey { pinned }))
        .with_client_cert_resolver(Arc::new(AlwaysResolvesClientRawPublicKeys::new(
            key.certified_key(),
        )));
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(Arc::new(config))
}

/// The pin of the key that a peer presented in a finished handshake.
fn peer_pin(keys: Option<&[CertificateDer<'_>]>) -> Option<KeyPin> {
    keys.and_then(<[CertificateDer<'_>]>::first)
        .map(|key| KeyPin::of_spki(key.as_ref()))
}

/// What a worker's server knows of a connection's peer.
#[derive(Debug, Clone)]
pub struct Peer {
    /// The pin of the key that the peer presented.
    pub pin: KeyPin,
    /// The connection, unique in this process.
    pub connection: u64,
}

/// A connection of the worker's server, with its peer.
#[derive(Debug)]
pub struct LinkStream {
    inner: tokio_rustls::server::TlsStream<TcpStream>,
    peer: Peer,
}

impl Connected for LinkStream {
    type ConnectInfo = Peer;

    fn connect_info(&self) -> Peer {
        self.peer.clone()
    }
}

impl AsyncRead for LinkStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(context, buffer)
    }
}

impl AsyncWrite for LinkStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffers: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(context, buffers)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

/// The connections of the worker's server: each accepted socket's handshake
/// runs on its own task, with at most [`MAX_HANDSHAKES`] at once and
/// [`HANDSHAKE_TIMEOUT`] each, and the connections whose handshakes finished
/// come out of the stream.
pub(crate) fn incoming(
    listener: TcpListener,
    config: Arc<ServerConfig>,
) -> BoxStream<'static, io::Result<LinkStream>> {
    incoming_with(listener, config, LIMITS)
}

fn incoming_with(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    limits: HandshakeLimits,
) -> BoxStream<'static, io::Result<LinkStream>> {
    let (sender, receiver) = mpsc::channel(limits.at_once);
    tokio::spawn(accept(listener, config, sender, limits));
    futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|stream| (Ok(stream), receiver))
    })
    .boxed()
}

/// Counts the connections of this process.
static CONNECTIONS: AtomicU64 = AtomicU64::new(0);

async fn accept(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    sender: mpsc::Sender<LinkStream>,
    limits: HandshakeLimits,
) {
    let acceptor = TlsAcceptor::from(config);
    let handshakes = Arc::new(Semaphore::new(limits.at_once));
    loop {
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            () = sender.closed() => return,
        };
        let tcp = match accepted {
            Ok((tcp, _address)) => tcp,
            Err(error) => {
                tracing::warn!(%error, "could not accept a connection of the link");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        // The semaphore is never closed.
        let Ok(permit) = Arc::clone(&handshakes).acquire_owned().await else {
            return;
        };
        let acceptor = acceptor.clone();
        let sender = sender.clone();
        tokio::spawn(async move {
            let shaken = tokio::time::timeout(limits.timeout, acceptor.accept(tcp)).await;
            drop(permit);
            let stream = match shaken {
                Ok(Ok(stream)) => stream,
                Ok(Err(error)) => {
                    tracing::debug!(%error, "a handshake of the link failed");
                    return;
                }
                Err(_elapsed) => {
                    tracing::debug!("a handshake of the link timed out");
                    return;
                }
            };
            let Some(pin) = peer_pin(stream.get_ref().1.peer_certificates()) else {
                return;
            };
            let peer = Peer {
                pin,
                connection: CONNECTIONS.fetch_add(1, Ordering::Relaxed),
            };
            let _sent = sender
                .send(LinkStream {
                    inner: stream,
                    peer,
                })
                .await;
        });
    }
}

/// Connects to a worker at `address`, `host:port`, and returns the TLS
/// stream with the pin of the key that the worker presented.
pub async fn connect(
    address: &str,
    config: Arc<ClientConfig>,
) -> io::Result<(tokio_rustls::client::TlsStream<TcpStream>, KeyPin)> {
    let connecting = async {
        let tcp = TcpStream::connect(address).await?;
        tcp.set_nodelay(true)?;
        let name = ServerName::try_from(WORKER_NAME)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        TlsConnector::from(config).connect(name, tcp).await
    };
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, connecting)
        .await
        .map_err(|_elapsed| io::Error::new(io::ErrorKind::TimedOut, "the handshake timed out"))??;
    let pin = peer_pin(stream.get_ref().1.peer_certificates())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "the worker presented no key"))?;
    Ok((stream, pin))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use futures_util::StreamExt;
    use tokio::io::AsyncReadExt;
    use tokio::net::{TcpListener, TcpStream};

    use super::{HandshakeLimits, client_config, connect, incoming_with, server_config};
    use crate::keys::LinkKey;

    async fn listen() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        (listener, address)
    }

    #[tokio::test]
    async fn a_stalled_handshake_is_cut_and_holds_its_place_until_then() {
        let worker = LinkKey::generate().unwrap();
        let master = LinkKey::generate().unwrap();
        let (listener, address) = listen().await;
        let limits = HandshakeLimits {
            timeout: Duration::from_millis(300),
            at_once: 1,
        };
        let mut connections = incoming_with(listener, server_config(&worker).unwrap(), limits);
        // A peer that opens a connection and never says anything takes the
        // only place.
        let mut stalled = TcpStream::connect(&address).await.unwrap();
        let started = Instant::now();

        let connecting = tokio::spawn({
            let config = client_config(&master, Some(worker.pin())).unwrap();
            async move { connect(&address, config).await }
        });
        let accepted = tokio::time::timeout(Duration::from_secs(5), connections.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let (_stream, worker_pin) = connecting.await.unwrap().unwrap();
        let mut byte = [0_u8; 1];
        let read = tokio::time::timeout(Duration::from_secs(5), stalled.read(&mut byte))
            .await
            .unwrap();

        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "the handshake waited for the stalled one: {:?}",
            started.elapsed()
        );
        assert_eq!(accepted.peer.pin, master.pin());
        assert_eq!(worker_pin, worker.pin());
        assert!(
            matches!(read, Ok(0) | Err(_)),
            "the stalled connection is closed: {read:?}"
        );
    }

    #[tokio::test]
    async fn the_master_refuses_a_worker_whose_key_is_not_the_pinned_one() {
        let worker = LinkKey::generate().unwrap();
        let other = LinkKey::generate().unwrap();
        let master = LinkKey::generate().unwrap();
        let (listener, address) = listen().await;
        let limits = HandshakeLimits {
            timeout: Duration::from_secs(5),
            at_once: 1,
        };
        let mut connections = incoming_with(listener, server_config(&worker).unwrap(), limits);

        let refused = connect(&address, client_config(&master, Some(other.pin())).unwrap()).await;
        let accepted = tokio::time::timeout(Duration::from_millis(500), connections.next()).await;

        refused.expect_err("the pinned key differs");
        accepted.expect_err("the worker got no connection");
    }
}
