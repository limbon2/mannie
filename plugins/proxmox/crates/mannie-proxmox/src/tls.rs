//! TLS configuration, OS trust verification, explicit SHA-256 leaf certificate
//! pinning, certificate inspection without credentials, and bounded ASN.1 DER
//! X.509 metadata parsing (ADR-0080, decision 3).

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use mannie_plugin_sdk::contracts::UtcMillis;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error, OtherError, SignatureScheme,
};
use sha2::{Digest as _, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::model::{CertificateInfo, ProxmoxError, format_fingerprint_colon};

/// Maximum certificate DER size accepted during inspection or pinning.
const MAX_CERT_DER_BYTES: usize = 64 * 1024;

/// Maximum SAN entries extracted from a leaf certificate.
const MAX_SANS: usize = 64;

/// Typed error attached to `CertificateError::Other` so callers can distinguish
/// leaf pin mismatches from general OS trust failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PinFailure {
    /// Leaf DER SHA-256 did not match the owner's pinned fingerprint.
    Mismatch,
    /// Leaf certificate could not be parsed as a valid X.509 certificate.
    Malformed,
}

impl fmt::Display for PinFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mismatch => f.write_str("leaf certificate SHA-256 does not match pinned digest"),
            Self::Malformed => f.write_str("leaf certificate DER is malformed"),
        }
    }
}

impl std::error::Error for PinFailure {}

/// Returns the `ring` crypto provider used across all Proxmox TLS connections.
pub(crate) fn ring_provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Computes the lowercase 64-character hex SHA-256 digest of a DER certificate.
pub(crate) fn sha256_hex(der: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(der);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// Builds a shared `rustls::ClientConfig` enforcing either OS trust
/// (`pinned_sha256 == None`) or exact leaf SHA-256 pinning (`Some(hex)`).
pub(crate) fn build_tls_config(
    pinned_sha256: Option<&str>,
    alpn: Vec<Vec<u8>>,
) -> Result<ClientConfig, ProxmoxError> {
    let provider = ring_provider();
    let verifier: Arc<dyn ServerCertVerifier> = if let Some(hex) = pinned_sha256 {
        let expected = decode_sha256_hex(hex).ok_or_else(|| ProxmoxError::InvalidRequest {
            detail: "invalid pinned SHA-256 fingerprint".to_owned(),
        })?;
        Arc::new(PinnedLeafCertVerifier {
            expected,
            algorithms: provider.signature_verification_algorithms,
        })
    } else {
        let os_verifier =
            rustls_platform_verifier::Verifier::new(Arc::clone(&provider)).map_err(|error| {
                tracing::warn!(%error, "could not initialize OS certificate verifier");
                ProxmoxError::ConnectionFailed {
                    detail: "could not initialize system certificate trust store".to_owned(),
                }
            })?;
        Arc::new(os_verifier)
    };

    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| ProxmoxError::ConnectionFailed {
            detail: format!("TLS protocol setup failed: {error}"),
        })?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = alpn;
    Ok(config)
}

/// Opens a TCP+TLS connection to `host:port` using `tls_config` and returns
/// the verified `TlsStream`.
pub(crate) async fn connect_tls(
    host: &str,
    port: u16,
    tls_config: Arc<ClientConfig>,
    timeout: Duration,
) -> Result<TlsStream<TcpStream>, ProxmoxError> {
    let server_name = server_name_for(host)?;
    let addr = format_socket_addr(host, port);
    let tcp = tokio::time::timeout(timeout, TcpStream::connect(&addr))
        .await
        .map_err(|_elapsed| ProxmoxError::Timeout)?
        .map_err(|error| ProxmoxError::ConnectionFailed {
            detail: error.to_string(),
        })?;
    let connector = TlsConnector::from(tls_config);
    tokio::time::timeout(timeout, connector.connect(server_name, tcp))
        .await
        .map_err(|_elapsed| ProxmoxError::Timeout)?
        .map_err(|error| classify_tls_io_error(&error))
}

/// Performs a TLS-only handshake to `host:port`, captures the presented leaf
/// certificate (plus intermediates), aborts before sending any application
/// bytes or credentials, parses X.509 metadata, and tests OS trust.
pub(crate) async fn probe_certificate(
    host: &str,
    port: u16,
    current_pin: Option<&str>,
    now: UtcMillis,
    timeout: Duration,
) -> Result<CertificateInfo, ProxmoxError> {
    let server_name = server_name_for(host)?;
    let addr = format_socket_addr(host, port);
    let tcp = tokio::time::timeout(timeout, TcpStream::connect(&addr))
        .await
        .map_err(|_elapsed| ProxmoxError::Timeout)?
        .map_err(|error| ProxmoxError::ConnectionFailed {
            detail: error.to_string(),
        })?;

    let provider = ring_provider();
    let captured: Arc<Mutex<Option<CapturedChain>>> = Arc::new(Mutex::new(None));
    let probe_verifier = Arc::new(ProbeCertVerifier {
        captured: Arc::clone(&captured),
        algorithms: provider.signature_verification_algorithms,
    });
    let config = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|error| ProxmoxError::CertificateInspectFailed {
            detail: error.to_string(),
        })?
        .dangerous()
        .with_custom_certificate_verifier(probe_verifier)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));

    // ProbeCertVerifier intentionally aborts inside verify_server_cert right
    // after capturing the presented chain, before TLS Finished.
    let handshake_result =
        tokio::time::timeout(timeout, connector.connect(server_name.clone(), tcp))
            .await
            .map_err(|_elapsed| ProxmoxError::Timeout)?;

    let chain = captured
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let Some(chain) = chain else {
        let detail = match handshake_result {
            Ok(_) => "server completed handshake without presenting a certificate".to_owned(),
            Err(error) => error.to_string(),
        };
        return Err(ProxmoxError::CertificateInspectFailed { detail });
    };

    if chain.leaf.len() > MAX_CERT_DER_BYTES {
        return Err(ProxmoxError::CertificateInspectFailed {
            detail: "presented certificate exceeds size limit".to_owned(),
        });
    }

    let parsed =
        parse_x509_metadata(&chain.leaf).ok_or_else(|| ProxmoxError::CertificateInspectFailed {
            detail: "could not parse X.509 leaf certificate".to_owned(),
        })?;
    let sha256 = sha256_hex(&chain.leaf);
    let fingerprint = format_fingerprint_colon(&sha256);
    let now_secs = now.as_millis() / 1000;
    let valid_now = now_secs >= parsed.not_before_unix && now_secs <= parsed.not_after_unix;

    let unix_now = UnixTime::since_unix_epoch(Duration::from_secs(
        u64::try_from(now_secs.max(0)).unwrap_or(0),
    ));
    let os_trusted = rustls_platform_verifier::Verifier::new(provider).is_ok_and(|verifier| {
        let leaf = CertificateDer::from(chain.leaf.as_slice());
        let intermediates: Vec<CertificateDer<'_>> = chain
            .intermediates
            .iter()
            .map(|bytes| CertificateDer::from(bytes.as_slice()))
            .collect();
        verifier
            .verify_server_cert(&leaf, &intermediates, &server_name, &[], unix_now)
            .is_ok()
    });
    let matches_pin = current_pin.map(|pin| pin == sha256);

    Ok(CertificateInfo {
        sha256,
        fingerprint,
        subject: parsed.subject,
        issuer: parsed.issuer,
        sans: parsed.sans,
        not_before: parsed.not_before_iso,
        not_after: parsed.not_after_iso,
        valid_now,
        os_trusted,
        matches_pin,
    })
}

/// Maps a `rustls::Error` (wrapped inside `std::io::Error` or `reqwest::Error`)
/// to the specific `ProxmoxError` certificate variant.
pub(crate) fn classify_rustls_error(error: &Error) -> Option<ProxmoxError> {
    match error {
        Error::InvalidCertificate(CertificateError::Expired | CertificateError::NotValidYet) => {
            Some(ProxmoxError::CertificateExpired)
        }
        Error::InvalidCertificate(CertificateError::Other(other)) => {
            if let Some(pin_err) = other.0.downcast_ref::<PinFailure>() {
                match pin_err {
                    PinFailure::Mismatch => Some(ProxmoxError::CertificateMismatch),
                    PinFailure::Malformed => Some(ProxmoxError::CertificateUntrusted),
                }
            } else {
                Some(ProxmoxError::CertificateUntrusted)
            }
        }
        Error::InvalidCertificate(_) => Some(ProxmoxError::CertificateUntrusted),
        Error::General(detail) => {
            let lower = detail.to_ascii_lowercase();
            let os_cert_reject = lower.contains("invalid algorithm")
                || lower.contains("certificate")
                || lower.contains("untrusted");
            os_cert_reject.then_some(ProxmoxError::CertificateUntrusted)
        }
        _ => None,
    }
}

/// Walks an `std::error::Error` source chain (including nested `std::io::Error`
/// wrappers) looking for a typed `rustls::Error` first, then falls back to
/// sanitized message classification for platform TLS wrappers.
pub(crate) fn classify_error_chain(
    error: &(dyn std::error::Error + 'static),
) -> Option<ProxmoxError> {
    if let Some(mapped) = find_typed_rustls_error(error) {
        return Some(mapped);
    }

    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(err) = current {
        if let Some(mapped) = classify_tls_message(&err.to_string()) {
            return Some(mapped);
        }

        current = err.source();
    }
    None
}

fn find_typed_rustls_error(error: &(dyn std::error::Error + 'static)) -> Option<ProxmoxError> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(err) = current {
        if let Some(tls_err) = err.downcast_ref::<Error>()
            && let Some(mapped) = classify_rustls_error(tls_err)
        {
            return Some(mapped);
        }

        if let Some(io_err) = err.downcast_ref::<std::io::Error>()
            && let Some(inner) = io_err.get_ref()
            && let Some(mapped) = find_typed_rustls_error(inner)
        {
            return Some(mapped);
        }

        current = err.source();
    }
    None
}

fn classify_tls_message(message: &str) -> Option<ProxmoxError> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("pinned") && lower.contains("certificate") {
        return Some(ProxmoxError::CertificateMismatch);
    }

    if lower.contains("certificate")
        && (lower.contains("expired")
            || lower.contains("not valid yet")
            || lower.contains("notvalidyet"))
    {
        return Some(ProxmoxError::CertificateExpired);
    }

    let is_cert_trust = lower.contains("invalid peer certificate")
        || lower.contains("unknownissuer")
        || lower.contains("unknown issuer")
        || lower.contains("self-signed")
        || lower.contains("untrusted")
        || lower.contains("certificate verify failed")
        || lower.contains("cert_e_untrustedroot")
        || lower.contains("not trusted");
    if is_cert_trust {
        return Some(ProxmoxError::CertificateUntrusted);
    }

    None
}

fn classify_tls_io_error(error: &std::io::Error) -> ProxmoxError {
    if let Some(mapped) = classify_error_chain(error) {
        return mapped;
    }

    ProxmoxError::ConnectionFailed {
        detail: error.to_string(),
    }
}

fn server_name_for(host: &str) -> Result<ServerName<'static>, ProxmoxError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ServerName::from(ip));
    }

    ServerName::try_from(host.to_owned()).map_err(|_err| ProxmoxError::InvalidRequest {
        detail: "invalid server hostname for TLS".to_owned(),
    })
}

fn format_socket_addr(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn decode_sha256_hex(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }

    let mut out = [0u8; 32];
    let bytes = hex.as_bytes();
    for (index, slot) in out.iter_mut().enumerate() {
        let hi = hex_nibble(bytes[index * 2])?;
        let lo = hex_nibble(bytes[index * 2 + 1])?;
        *slot = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn constant_time_eq_32(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for index in 0..32 {
        diff |= left[index] ^ right[index];
    }
    diff == 0
}

/// Custom `ServerCertVerifier` verifying exact leaf SHA-256 fingerprint, X.509
/// validity window, and TLS 1.2 / 1.3 handshake signatures via `ring`.
/// Works for both Proxmox cluster-CA-signed node certs and self-signed certs.
#[derive(Debug)]
struct PinnedLeafCertVerifier {
    expected: [u8; 32],
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedLeafCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let der = end_entity.as_ref();
        if der.len() > MAX_CERT_DER_BYTES {
            return Err(Error::InvalidCertificate(CertificateError::Other(
                OtherError(Arc::new(PinFailure::Malformed)),
            )));
        }

        let actual: [u8; 32] = Sha256::digest(der).into();
        if !constant_time_eq_32(&actual, &self.expected) {
            return Err(Error::InvalidCertificate(CertificateError::Other(
                OtherError(Arc::new(PinFailure::Mismatch)),
            )));
        }

        // Validate that webpki can parse the leaf certificate's SPKI for signature checks.
        if webpki::EndEntityCert::try_from(end_entity).is_err() {
            return Err(Error::InvalidCertificate(CertificateError::Other(
                OtherError(Arc::new(PinFailure::Malformed)),
            )));
        }

        let metadata = parse_x509_metadata(der).ok_or_else(|| {
            Error::InvalidCertificate(CertificateError::Other(OtherError(Arc::new(
                PinFailure::Malformed,
            ))))
        })?;
        let now_secs = i64::try_from(now.as_secs()).unwrap_or(i64::MAX);
        if now_secs < metadata.not_before_unix {
            return Err(Error::InvalidCertificate(CertificateError::NotValidYet));
        }

        if now_secs > metadata.not_after_unix {
            return Err(Error::InvalidCertificate(CertificateError::Expired));
        }

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[derive(Debug)]
struct CapturedChain {
    leaf: Vec<u8>,
    intermediates: Vec<Vec<u8>>,
}

/// Captures the presented certificate chain during TLS handshake and aborts
/// immediately so no credentials or application data are ever transmitted.
#[derive(Debug)]
struct ProbeCertVerifier {
    captured: Arc<Mutex<Option<CapturedChain>>>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for ProbeCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let mut slot = self.captured.lock().unwrap_or_else(PoisonError::into_inner);
        *slot = Some(CapturedChain {
            leaf: end_entity.as_ref().to_vec(),
            intermediates: intermediates
                .iter()
                .map(|cert| cert.as_ref().to_vec())
                .collect(),
        });
        Err(Error::General("proxmox_probe_complete".to_owned()))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

// ---------------------------------------------------------------------------
// Minimal zero-unsafe ASN.1 DER X.509 v3 metadata parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedX509 {
    pub(crate) subject: String,
    pub(crate) issuer: String,
    pub(crate) sans: Vec<String>,
    pub(crate) not_before_unix: i64,
    pub(crate) not_after_unix: i64,
    pub(crate) not_before_iso: String,
    pub(crate) not_after_iso: String,
}

/// Parses Subject, Issuer, Validity (`notBefore`, `notAfter`), and
/// `SubjectAlternativeName` (OID 2.5.29.17) from a DER-encoded X.509 certificate.
pub(crate) fn parse_x509_metadata(der: &[u8]) -> Option<ParsedX509> {
    let mut outer = DerReader::new(der);
    let cert_body = outer.expect_tag(0x30)?;
    let mut cert = DerReader::new(cert_body);
    let tbs_body = cert.expect_tag(0x30)?;
    let mut tbs = DerReader::new(tbs_body);

    // Optional explicit [0] version
    if tbs.peek_tag() == Some(0xa0) {
        let _version = tbs.read_tlv()?;
    }

    // serialNumber (INTEGER)
    let _serial = tbs.expect_tag(0x02)?;
    // signature (AlgorithmIdentifier SEQUENCE)
    let _sig_alg = tbs.expect_tag(0x30)?;
    // issuer (Name SEQUENCE)
    let issuer_der = tbs.expect_tag(0x30)?;
    // validity (Validity SEQUENCE)
    let validity_der = tbs.expect_tag(0x30)?;
    // subject (Name SEQUENCE)
    let subject_der = tbs.expect_tag(0x30)?;
    // subjectPublicKeyInfo (SEQUENCE)
    let _spki = tbs.expect_tag(0x30)?;

    let issuer = parse_name(issuer_der).unwrap_or_default();
    let subject = parse_name(subject_der).unwrap_or_default();
    let (not_before_unix, not_before_iso, not_after_unix, not_after_iso) =
        parse_validity(validity_der)?;

    let mut sans = Vec::new();
    while !tbs.is_empty() {
        let (tag, content) = tbs.read_tlv()?;
        // Explicit [3] extensions
        if tag == 0xa3 {
            sans = parse_extensions_for_san(content);
            break;
        }
    }

    Some(ParsedX509 {
        subject,
        issuer,
        sans,
        not_before_unix,
        not_after_unix,
        not_before_iso,
        not_after_iso,
    })
}

struct DerReader<'a> {
    data: &'a [u8],
}

impl<'a> DerReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn peek_tag(&self) -> Option<u8> {
        self.data.first().copied()
    }

    fn expect_tag(&mut self, expected: u8) -> Option<&'a [u8]> {
        let (tag, content) = self.read_tlv()?;
        (tag == expected).then_some(content)
    }

    fn read_tlv(&mut self) -> Option<(u8, &'a [u8])> {
        let tag = *self.data.first()?;
        let rest = &self.data[1..];
        let first_len = *rest.first()?;
        let (len, header_len) = if first_len < 0x80 {
            (usize::from(first_len), 1usize)
        } else {
            let num_bytes = usize::from(first_len & 0x7f);
            if num_bytes == 0 || num_bytes > 4 || rest.len() < 1 + num_bytes {
                return None;
            }

            let mut length = 0usize;
            for &byte in &rest[1..=num_bytes] {
                length = (length << 8) | usize::from(byte);
            }
            (length, 1 + num_bytes)
        };

        let body_start = header_len;
        let body_end = body_start.checked_add(len)?;
        if body_end > rest.len() {
            return None;
        }

        let content = &rest[body_start..body_end];
        self.data = &rest[body_end..];
        Some((tag, content))
    }
}

fn parse_name(name_der: &[u8]) -> Option<String> {
    let mut reader = DerReader::new(name_der);
    let mut parts = Vec::new();

    while !reader.is_empty() {
        let rdn_set = reader.expect_tag(0x31)?;
        let mut rdn = DerReader::new(rdn_set);
        while !rdn.is_empty() {
            let atv = rdn.expect_tag(0x30)?;
            let mut fields = DerReader::new(atv);
            let oid = fields.expect_tag(0x06)?;
            let (_val_tag, val_bytes) = fields.read_tlv()?;
            let key = oid_short_name(oid);
            let value = String::from_utf8_lossy(val_bytes)
                .chars()
                .filter(|ch| !ch.is_control())
                .collect::<String>();
            if !value.is_empty() {
                parts.push(format!("{key}={value}"));
            }
        }
    }

    Some(parts.join(", "))
}

fn oid_short_name(oid: &[u8]) -> &'static str {
    match oid {
        [0x55, 0x04, 0x03] => "CN",
        [0x55, 0x04, 0x06] => "C",
        [0x55, 0x04, 0x07] => "L",
        [0x55, 0x04, 0x08] => "ST",
        [0x55, 0x04, 0x0a] => "O",
        [0x55, 0x04, 0x0b] => "OU",
        _ => "OID",
    }
}

fn parse_validity(validity_der: &[u8]) -> Option<(i64, String, i64, String)> {
    let mut reader = DerReader::new(validity_der);
    let (tag_before, bytes_before) = reader.read_tlv()?;
    let (tag_after, bytes_after) = reader.read_tlv()?;
    let (before_unix, before_iso) = parse_asn1_time(tag_before, bytes_before)?;
    let (after_unix, after_iso) = parse_asn1_time(tag_after, bytes_after)?;
    Some((before_unix, before_iso, after_unix, after_iso))
}

fn parse_asn1_time(tag: u8, bytes: &[u8]) -> Option<(i64, String)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let (year, rest) = match tag {
        // UTCTime: YYMMDDHHMMSSZ (RFC 5280: 00..=49 -> 2000..=2049, 50..=99 -> 1950..=1999)
        0x17 => {
            if text.len() < 13 || !text.ends_with('Z') {
                return None;
            }

            let yy: i32 = text[0..2].parse().ok()?;
            let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
            (year, &text[2..text.len() - 1])
        }
        // GeneralizedTime: YYYYMMDDHHMMSSZ
        0x18 => {
            if text.len() < 15 || !text.ends_with('Z') {
                return None;
            }

            let year: i32 = text[0..4].parse().ok()?;
            (year, &text[4..text.len() - 1])
        }
        _ => return None,
    };

    if rest.len() < 10 {
        return None;
    }

    let month: u32 = rest[0..2].parse().ok()?;
    let day: u32 = rest[2..4].parse().ok()?;
    let hour: u32 = rest[4..6].parse().ok()?;
    let minute: u32 = rest[6..8].parse().ok()?;
    let second: u32 = rest[8..10].parse().ok()?;

    let date_ok = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour <= 23
        && minute <= 59
        && second <= 60;
    if !date_ok {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let unix = days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    let iso = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z");
    Some((unix, iso))
}

/// Howard Hinnant's civil-from-days inverse: days since 1970-01-01.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// OID 2.5.29.17 (subjectAltName) DER value bytes.
const OID_SAN: &[u8] = &[0x55, 0x1d, 0x11];

fn parse_extensions_for_san(ext_wrapper: &[u8]) -> Vec<String> {
    let mut wrapper = DerReader::new(ext_wrapper);
    let Some(seq_bytes) = wrapper.expect_tag(0x30) else {
        return Vec::new();
    };
    let mut seq = DerReader::new(seq_bytes);

    while !seq.is_empty() {
        let Some(ext_bytes) = seq.expect_tag(0x30) else {
            break;
        };
        let mut ext = DerReader::new(ext_bytes);
        let Some(oid) = ext.expect_tag(0x06) else {
            continue;
        };

        // Optional critical BOOLEAN (0x01)
        if ext.peek_tag() == Some(0x01) {
            let _ = ext.read_tlv();
        }

        let Some(extn_value) = ext.expect_tag(0x04) else {
            continue;
        };

        if oid == OID_SAN {
            return parse_san_sequence(extn_value);
        }
    }

    Vec::new()
}

fn parse_san_sequence(san_der: &[u8]) -> Vec<String> {
    let mut outer = DerReader::new(san_der);
    let Some(seq_bytes) = outer.expect_tag(0x30) else {
        return Vec::new();
    };
    let mut seq = DerReader::new(seq_bytes);
    let mut sans = Vec::new();

    while !seq.is_empty() && sans.len() < MAX_SANS {
        let Some((tag, value)) = seq.read_tlv() else {
            break;
        };
        match tag {
            // dNSName [2] IA5String
            0x82 => {
                if let Ok(dns) = std::str::from_utf8(value) {
                    let clean: String = dns.chars().filter(|ch| !ch.is_control()).collect();
                    if !clean.is_empty() {
                        sans.push(clean);
                    }
                }
            }
            // iPAddress [7] OCTET STRING (4 bytes IPv4 or 16 bytes IPv6)
            0x87 => {
                if let Ok(octets) = <[u8; 4]>::try_from(value) {
                    sans.push(Ipv4Addr::from(octets).to_string());
                } else if let Ok(octets) = <[u8; 16]>::try_from(value) {
                    sans.push(Ipv6Addr::from(octets).to_string());
                }
            }
            _ => {}
        }
    }

    sans
}

#[cfg(test)]
mod tests {
    use super::{days_from_civil, decode_sha256_hex, parse_asn1_time};

    #[test]
    fn civil_date_matches_unix_epoch_and_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 1, 1), 10_957);
        assert_eq!(days_from_civil(2026, 11, 24), 20_781);
    }

    #[test]
    fn asn1_time_parses_utc_and_generalized_time() {
        let (unix, iso) = parse_asn1_time(0x17, b"261124120000Z").unwrap();
        assert_eq!(iso, "2026-11-24T12:00:00Z");
        assert_eq!(unix, 20_781 * 86_400 + 12 * 3_600);

        let (unix2, iso2) = parse_asn1_time(0x18, b"20300101000000Z").unwrap();
        assert_eq!(iso2, "2030-01-01T00:00:00Z");
        assert!(unix2 > unix);
    }

    #[test]
    fn decodes_valid_sha256_hex_only() {
        let decoded = decode_sha256_hex(&"ab".repeat(32)).unwrap();
        assert_eq!(decoded, [0xab; 32]);
        assert!(decode_sha256_hex("short").is_none());
    }
}
