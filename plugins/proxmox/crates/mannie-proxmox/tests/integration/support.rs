//! In-process loopback fake Proxmox VE HTTPS + RFC 6455 WSS + RFB 3.8 server
//! with dynamic Ed25519 X.509 v3 certificate generation (self-signed, CA-signed,
//! rotated, and expired) and planted-secret tracking.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use base64::Engine as _;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_harness::runtime::HarnessOptions;
use mannie_plugin_sdk::plugin::PluginEntry;
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_proxmox::{PLUGIN_MANIFEST, ProxmoxPlugin, mannie_plugin};
use ring::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as _};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

pub(crate) const USERNAME: &str = "lab@pve";
pub(crate) const PASSWORD: &str = "PLANTED_PROXMOX_PASSWORD_9f8e7d6c";
pub(crate) const TICKET: &str = "PVE:lab@pve:PLANTED_TICKET_SECRET_11223344::sig";
pub(crate) const CSRF: &str = "PLANTED_CSRF_SECRET_55667788";
pub(crate) const VNC_TICKET: &str = "VNCTKT99_PLANTED_SECRET";

/// RFB encoding mode served by the fake VNC endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RfbMode {
    Raw,
    Zlib,
    CopyRect,
}

/// One simulated QEMU VM hosted by `FakeProxmoxServer`.
#[derive(Debug, Clone)]
pub(crate) struct FakeVm {
    pub(crate) vmid: u32,
    pub(crate) node: String,
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) config: BTreeMap<String, String>,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) pixels: Vec<u8>,
    pub(crate) rfb_mode: RfbMode,
}

impl FakeVm {
    pub(crate) fn new(vmid: u32, node: &str, name: &str, status: &str) -> Self {
        let width: u16 = 64;
        let height: u16 = 48;
        let mut pixels = vec![0u8; usize::from(width) * usize::from(height) * 4];
        for px in pixels.as_chunks_mut::<4>().0 {
            px[0] = 30;
            px[1] = 60;
            px[2] = 120;
            px[3] = 255;
        }
        let mut config = BTreeMap::new();
        config.insert("name".to_owned(), name.to_owned());
        config.insert("cores".to_owned(), "2".to_owned());
        config.insert("memory".to_owned(), "2048".to_owned());
        config.insert("boot".to_owned(), "order=scsi0;ide2".to_owned());
        config.insert("ide2".to_owned(), "none,media=cdrom".to_owned());
        config.insert(
            "scsi0".to_owned(),
            "local-lvm:vm-disk-0,size=20G".to_owned(),
        );
        Self {
            vmid,
            node: node.to_owned(),
            name: name.to_owned(),
            status: status.to_owned(),
            config,
            width,
            height,
            pixels,
            rfb_mode: RfbMode::Raw,
        }
    }
}

/// Generated Ed25519 X.509 v3 certificate bundle for the fake TLS server.
#[derive(Debug, Clone)]
pub(crate) struct TestCertBundle {
    pub(crate) sha256: String,
    pub(crate) config: Arc<ServerConfig>,
}

/// Mutable server state inspected by assertions.
#[derive(Debug)]
pub(crate) struct FakeState {
    pub(crate) vms: Vec<FakeVm>,
    pub(crate) isos: BTreeMap<String, Vec<(String, u64)>>,
    pub(crate) tfa_required: bool,
    pub(crate) auth_attempts: usize,
    pub(crate) config_updates: Vec<(u32, BTreeMap<String, String>)>,
    pub(crate) power_actions: Vec<(u32, String)>,
    pub(crate) vnc_pointer_events: Vec<(u32, u16, u16, u8)>,
    pub(crate) vnc_key_events: Vec<(u32, u32, bool)>,
    pub(crate) ticket_set_cookies: Vec<String>,
    pub(crate) vncproxy_set_cookies: Vec<String>,
    pub(crate) require_ws_cookies: Vec<String>,
    pub(crate) ws_error_response: Option<String>,
    pub(crate) last_ws_headers: Option<BTreeMap<String, String>>,
    pub(crate) last_ws_alpn: Option<Vec<u8>>,
    vnc_key_event_received: Arc<Notify>,
}

impl Default for FakeState {
    fn default() -> Self {
        let mut isos = BTreeMap::new();
        isos.insert(
            "local".to_owned(),
            vec![
                (
                    "local:iso/debian-13-netinst.iso".to_owned(),
                    680 * 1024 * 1024,
                ),
                (
                    "local:iso/ubuntu-24.04-server.iso".to_owned(),
                    1_400 * 1024 * 1024,
                ),
            ],
        );
        Self {
            vms: vec![
                FakeVm::new(100, "pve1", "debian-lab", "running"),
                FakeVm::new(101, "pve1", "windows-sandbox", "stopped"),
                FakeVm::new(200, "pve2", "prod-secret-db", "running"),
            ],
            isos,
            tfa_required: false,
            auth_attempts: 0,
            config_updates: Vec::new(),
            power_actions: Vec::new(),
            vnc_pointer_events: Vec::new(),
            vnc_key_events: Vec::new(),
            ticket_set_cookies: Vec::new(),
            vncproxy_set_cookies: Vec::new(),
            require_ws_cookies: Vec::new(),
            ws_error_response: None,
            last_ws_headers: None,
            last_ws_alpn: None,
            vnc_key_event_received: Arc::new(Notify::new()),
        }
    }
}

/// Running loopback fake Proxmox VE HTTPS + WSS + RFB 3.8 test server.
pub(crate) struct FakeProxmoxServer {
    pub(crate) addr: SocketAddr,
    pub(crate) state: Arc<Mutex<FakeState>>,
    tls_config: Arc<RwLock<Arc<ServerConfig>>>,
    cert_sha256: Arc<RwLock<String>>,
    cert_gen: tokio::sync::watch::Sender<u64>,
    pub(crate) active_vnc_sockets: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl FakeProxmoxServer {
    pub(crate) async fn start(cert: TestCertBundle) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake Proxmox listener");
        let addr = listener.local_addr().expect("listener local addr");
        let state = Arc::new(Mutex::new(FakeState::default()));
        let tls_config = Arc::new(RwLock::new(Arc::clone(&cert.config)));
        let cert_sha256 = Arc::new(RwLock::new(cert.sha256.clone()));
        let (cert_gen, _) = tokio::sync::watch::channel(0u64);
        let active_vnc_sockets = Arc::new(AtomicUsize::new(0));

        let task_state = Arc::clone(&state);
        let task_tls = Arc::clone(&tls_config);
        let task_gen = cert_gen.clone();
        let task_vnc = Arc::clone(&active_vnc_sockets);
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _peer)) = listener.accept().await else {
                    return;
                };
                let current_cfg = Arc::clone(&task_tls.read().unwrap());
                let acceptor = TlsAcceptor::from(current_cfg);
                let conn_state = Arc::clone(&task_state);
                let mut conn_gen = task_gen.subscribe();
                let conn_vnc = Arc::clone(&task_vnc);
                tokio::spawn(async move {
                    let Ok(tls_stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let negotiated_alpn =
                        tls_stream.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
                    tokio::select! {
                        res = handle_connection(tls_stream, negotiated_alpn, conn_state, conn_vnc) => {
                            drop(res);
                        }
                        _ = conn_gen.changed() => {
                            // Server certificate rotated: drop old keep-alive connection.
                        }
                    }
                });
            }
        });

        Self {
            addr,
            state,
            tls_config,
            cert_sha256,
            cert_gen,
            active_vnc_sockets,
            task,
        }
    }

    pub(crate) fn port(&self) -> u16 {
        self.addr.port()
    }

    pub(crate) fn sha256(&self) -> String {
        self.cert_sha256.read().unwrap().clone()
    }

    pub(crate) fn rotate_cert(&self, cert: TestCertBundle) {
        *self.tls_config.write().unwrap() = cert.config;
        *self.cert_sha256.write().unwrap() = cert.sha256;
        let next = self.cert_gen.borrow().wrapping_add(1);
        let _prev = self.cert_gen.send_replace(next);
    }

    pub(crate) fn auth_attempts(&self) -> usize {
        self.state.lock().unwrap().auth_attempts
    }

    pub(crate) async fn wait_for_vnc_key_events(
        &self,
        vmid: u32,
        count: usize,
    ) -> Vec<(u32, bool)> {
        let received = Arc::clone(&self.state.lock().unwrap().vnc_key_event_received);

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                // Subscribe before reading the events so a concurrent update cannot be missed.
                let notified = received.notified();
                let events: Vec<_> = self
                    .state
                    .lock()
                    .unwrap()
                    .vnc_key_events
                    .iter()
                    .filter(|&&(id, _, _)| id == vmid)
                    .map(|&(_, keysym, down)| (keysym, down))
                    .collect();

                if events.len() >= count {
                    return events;
                }

                notified.await;
            }
        })
        .await
        .expect("fake Proxmox server receives the expected VNC key events")
    }
}

impl Drop for FakeProxmoxServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Generates a fresh synthetic `ConversationId` for direct IPC tests.
pub(crate) fn new_conv_id() -> mannie_plugin_sdk::contracts::ConversationId {
    serde_json::from_value(json!(uuid::Uuid::now_v7().to_string())).expect("valid conversation id")
}

/// Hosts `mannie-proxmox` on the real plugin host with in-memory secrets.
pub(crate) async fn hosted_plugin(secrets: &FakeSecrets, use_tools: bool) -> HostedPlugin {
    let core = HarnessOptions {
        use_tools,
        ..HarnessOptions::default()
    };
    let options = HostOptions::new()
        .with_core(core)
        .with_provider(Arc::new(secrets.clone()));
    let hosted = HostedPlugin::new(mannie_plugin(), options).await;
    hosted.turn_on().await;
    hosted
}

/// Hosts `mannie-proxmox` with a short VNC idle timeout for reaper testing.
pub(crate) async fn hosted_plugin_with_idle_timeout(
    secrets: &FakeSecrets,
    idle_timeout: std::time::Duration,
) -> HostedPlugin {
    let entry = PluginEntry::new(
        PLUGIN_MANIFEST,
        Arc::new(ProxmoxPlugin::with_idle_timeout(idle_timeout)),
    );
    let options = HostOptions::new().with_provider(Arc::new(secrets.clone()));
    let hosted = HostedPlugin::new(entry, options).await;
    hosted.turn_on().await;
    hosted
}

/// Adds a server pointing at `fake`, inspects and pins its leaf cert, and
/// optionally allows the listed VMIDs.
pub(crate) async fn register_pinned_server(
    hosted: &HostedPlugin,
    name: &str,
    fake: &FakeProxmoxServer,
    allowed_vmids: &[u32],
) -> Value {
    let added = hosted
        .invoke(
            "add_server",
            json!({
                "name": name,
                "host": "127.0.0.1",
                "port": fake.port(),
                "username": USERNAME,
                "password": PASSWORD,
                "note": "Test lab server",
            }),
        )
        .await
        .expect("add_server");
    let server_id = added["id"].as_str().unwrap().to_owned();

    let cert = hosted
        .invoke("inspect_certificate", json!({ "serverId": server_id }))
        .await
        .expect("inspect_certificate");
    let sha256 = cert["sha256"].as_str().unwrap();

    let mut view = hosted
        .invoke(
            "pin_certificate",
            json!({ "serverId": server_id, "sha256": sha256 }),
        )
        .await
        .expect("pin_certificate");

    if !allowed_vmids.is_empty() {
        let discoverable = hosted
            .invoke("discover_server_vms", json!({ "serverId": server_id }))
            .await
            .expect("discover_server_vms");
        let vms: Vec<Value> = discoverable
            .as_array()
            .unwrap()
            .iter()
            .filter(|vm| {
                allowed_vmids.contains(&u32::try_from(vm["vmid"].as_u64().unwrap()).unwrap())
            })
            .map(|vm| {
                json!({
                    "vmid": vm["vmid"],
                    "node": vm["node"],
                    "name": vm["name"],
                })
            })
            .collect();
        view = hosted
            .invoke(
                "set_allowed_vms",
                json!({ "serverId": server_id, "vms": vms }),
            )
            .await
            .expect("set_allowed_vms");
    }

    view
}

// ---------------------------------------------------------------------------
// Pure-Rust Ed25519 X.509 v3 DER certificate builder using ring
// ---------------------------------------------------------------------------

pub(crate) fn build_ed25519_cert(
    subject_cn: &str,
    issuer_cn: &str,
    not_before_utc: &str,
    not_after_utc: &str,
    serial_byte: u8,
) -> TestCertBundle {
    drop(rustls::crypto::ring::default_provider().install_default());
    let rng = SystemRandom::new();
    let leaf_pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let leaf_kp =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, leaf_pkcs8.as_ref(), &rng)
            .unwrap();

    // If issuer != subject, sign the leaf with a separate simulated cluster CA key
    // so the test exercises real Proxmox cluster-CA-signed leaf certificates.
    let signer_pkcs8 = if issuer_cn == subject_cn {
        None
    } else {
        Some(EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap())
    };
    let signer_kp = signer_pkcs8.as_ref().map(|pkcs8| {
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap()
    });
    let signer = signer_kp.as_ref().unwrap_or(&leaf_kp);

    // ECDSA with SHA-256 (OID 1.2.840.10045.4.3.2)
    let sig_alg = der_seq(&der_oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02]));
    // id-ecPublicKey (1.2.840.10045.2.1) + prime256v1 / secp256r1 (1.2.840.10045.3.1.7)
    let ec_alg = der_seq(
        &[
            der_oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]),
            der_oid(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]),
        ]
        .concat(),
    );
    let version = der_explicit(0, &der_int(&[2]));
    let serial = der_int(&[serial_byte.max(1)]);
    let issuer = der_cn_name(issuer_cn);
    let subject = der_cn_name(subject_cn);
    let validity = der_seq(&[der_utc_time(not_before_utc), der_utc_time(not_after_utc)].concat());
    let spki = der_seq(&[ec_alg, der_bit_string(leaf_kp.public_key().as_ref())].concat());

    // Extensions: BasicConstraints (CA=false), KeyUsage (digitalSignature), EKU (serverAuth), SAN (localhost + 127.0.0.1)
    let bc_ext = der_extension(&[0x55, 0x1d, 0x13], true, &der_seq(&[]));
    let ku_ext = der_extension(
        &[0x55, 0x1d, 0x0f],
        true,
        &der_bit_string_unused(7, &[0x80]),
    );
    let eku_ext = der_extension(
        &[0x55, 0x1d, 0x25],
        false,
        &der_seq(&der_oid(&[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01])),
    );
    let san_dns = der_tlv(0x82, b"localhost");
    let san_ip = der_tlv(0x87, &[127, 0, 0, 1]);
    let san_ext = der_extension(
        &[0x55, 0x1d, 0x11],
        false,
        &der_seq(&[san_dns, san_ip].concat()),
    );
    let extensions = der_explicit(3, &der_seq(&[bc_ext, ku_ext, eku_ext, san_ext].concat()));

    let tbs = der_seq(
        &[
            version,
            serial,
            sig_alg.clone(),
            issuer,
            validity,
            subject,
            spki,
            extensions,
        ]
        .concat(),
    );
    let sig = signer.sign(&rng, &tbs).unwrap();
    let cert_der = der_seq(&[tbs, sig_alg, der_bit_string(sig.as_ref())].concat());

    let mut hasher = Sha256::new();
    hasher.update(&cert_der);
    let sha256 = hex_lower(&hasher.finalize());

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_pkcs8.as_ref().to_vec())),
        )
        .expect("build rustls ServerConfig");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    TestCertBundle {
        sha256,
        config: Arc::new(config),
    }
}

pub(crate) fn valid_self_signed_cert(serial: u8) -> TestCertBundle {
    build_ed25519_cert(
        "pve.local",
        "pve.local",
        "250101000000Z",
        "350101000000Z",
        serial,
    )
}

pub(crate) fn valid_cluster_ca_signed_cert(serial: u8) -> TestCertBundle {
    build_ed25519_cert(
        "pve1.cluster.lan",
        "Proxmox Virtual Environment Cluster CA",
        "250101000000Z",
        "350101000000Z",
        serial,
    )
}

pub(crate) fn expired_cert(serial: u8) -> TestCertBundle {
    build_ed25519_cert(
        "pve-old.local",
        "pve-old.local",
        "200101000000Z",
        "210101000000Z",
        serial,
    )
}

fn der_tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(tag);
    if body.len() < 0x80 {
        out.push(u8::try_from(body.len()).unwrap());
    } else if let Ok(short_len) = u8::try_from(body.len()) {
        out.push(0x81);
        out.push(short_len);
    } else {
        let len16 = u16::try_from(body.len()).unwrap();
        out.push(0x82);
        out.extend_from_slice(&len16.to_be_bytes());
    }
    out.extend_from_slice(body);
    out
}

fn der_seq(body: &[u8]) -> Vec<u8> {
    der_tlv(0x30, body)
}

fn der_set(body: &[u8]) -> Vec<u8> {
    der_tlv(0x31, body)
}

fn der_int(bytes: &[u8]) -> Vec<u8> {
    der_tlv(0x02, bytes)
}

fn der_oid(bytes: &[u8]) -> Vec<u8> {
    der_tlv(0x06, bytes)
}

fn der_bit_string(bytes: &[u8]) -> Vec<u8> {
    der_bit_string_unused(0, bytes)
}

fn der_bit_string_unused(unused: u8, bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(bytes.len() + 1);
    body.push(unused);
    body.extend_from_slice(bytes);
    der_tlv(0x03, &body)
}

fn der_explicit(tag_num: u8, body: &[u8]) -> Vec<u8> {
    der_tlv(0xa0 | tag_num, body)
}

fn der_utc_time(text: &str) -> Vec<u8> {
    der_tlv(0x17, text.as_bytes())
}

fn der_cn_name(cn: &str) -> Vec<u8> {
    let attr = der_seq(&[der_oid(&[0x55, 0x04, 0x03]), der_tlv(0x0c, cn.as_bytes())].concat());
    der_seq(&der_set(&attr))
}

fn der_extension(oid: &[u8], critical: bool, value: &[u8]) -> Vec<u8> {
    let mut inner = der_oid(oid);
    if critical {
        inner.extend_from_slice(&der_tlv(0x01, &[0xff]));
    }
    inner.extend_from_slice(&der_tlv(0x04, value));
    der_seq(&inner)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{b:02x}").unwrap();
    }
    out
}

// ---------------------------------------------------------------------------
// HTTP/1.1 + WebSocket + RFB 3.8 protocol handler
// ---------------------------------------------------------------------------

async fn handle_connection<S>(
    mut stream: S,
    negotiated_alpn: Option<Vec<u8>>,
    state: Arc<Mutex<FakeState>>,
    active_vnc_sockets: Arc<AtomicUsize>,
) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let Some(req) = read_http_request(&mut stream).await? else {
            return Ok(());
        };

        if req.is_websocket_upgrade() {
            handle_vnc_websocket(stream, req, negotiated_alpn, state, active_vnc_sockets).await?;
            return Ok(());
        }

        let (status, body, set_cookies) = route_api_request(&req, &state);
        write_http_json(&mut stream, status, &body, &set_cookies).await?;
    }
}

struct HttpRequest {
    method: String,
    path: String,
    query: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    fn is_websocket_upgrade(&self) -> bool {
        self.header("upgrade")
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
    }

    fn has_valid_cookie(&self) -> bool {
        self.header("cookie")
            .is_some_and(|cookie| cookie.contains(&format!("PVEAuthCookie={TICKET}")))
    }

    fn has_valid_csrf(&self) -> bool {
        self.header("csrfpreventiontoken")
            .is_some_and(|token| token == CSRF)
    }
}

async fn read_http_request<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> std::io::Result<Option<HttpRequest>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated HTTP request headers",
            ));
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default();
    let (path, query) = target.split_once('?').map_or_else(
        || (target.to_owned(), String::new()),
        |(p, q)| (p.to_owned(), q.to_owned()),
    );

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, val)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), val.trim().to_owned());
        }
    }

    let content_len = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; content_len];
    if content_len > 0 {
        stream.read_exact(&mut body).await?;
    }

    Ok(Some(HttpRequest {
        method,
        path,
        query,
        headers,
        body,
    }))
}

async fn write_http_json<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status: u16,
    body: &Value,
    set_cookies: &[String],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let json_bytes = serde_json::to_vec(body).unwrap();
    let mut header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n",
        json_bytes.len()
    );
    for cookie in set_cookies {
        header.push_str("Set-Cookie: ");
        header.push_str(cookie);
        header.push_str("\r\n");
    }
    header.push_str("\r\n");
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&json_bytes).await?;
    stream.flush().await
}

#[expect(
    clippy::too_many_lines,
    reason = "fake Proxmox VE REST API router handles all test endpoints in one table"
)]
fn route_api_request(
    req: &HttpRequest,
    state: &Arc<Mutex<FakeState>>,
) -> (u16, Value, Vec<String>) {
    if req.method == "POST" && req.path == "/api2/json/access/ticket" {
        let form: BTreeMap<String, String> = url::form_urlencoded::parse(&req.body)
            .into_owned()
            .collect();
        let mut guard = state.lock().unwrap();
        guard.auth_attempts += 1;
        let user_ok = form.get("username").map(String::as_str) == Some(USERNAME);
        let pass_ok = form.get("password").map(String::as_str) == Some(PASSWORD);
        if !user_ok || !pass_ok {
            return (401, json!({ "data": null }), Vec::new());
        }
        if guard.tfa_required {
            return (
                200,
                json!({
                    "data": {
                        "NeedTFA": 1,
                        "ticket": "PVE:!tfa!challenge",
                        "CSRFPreventionToken": CSRF,
                        "username": USERNAME,
                    }
                }),
                Vec::new(),
            );
        }
        return (
            200,
            json!({
                "data": {
                    "ticket": TICKET,
                    "CSRFPreventionToken": CSRF,
                    "username": USERNAME,
                }
            }),
            guard.ticket_set_cookies.clone(),
        );
    }

    if !req.has_valid_cookie() {
        return (401, json!({ "data": null }), Vec::new());
    }

    if (req.method == "POST" || req.method == "PUT") && !req.has_valid_csrf() {
        return (401, json!({ "data": null }), Vec::new());
    }

    let mut guard = state.lock().unwrap();

    if req.method == "GET" && req.path == "/api2/json/cluster/resources" {
        let mut items: Vec<Value> = guard
            .vms
            .iter()
            .map(|vm| {
                json!({
                    "type": "qemu",
                    "vmid": vm.vmid,
                    "node": vm.node,
                    "name": vm.name,
                    "status": vm.status,
                    "maxcpu": 2,
                    "maxmem": 2_147_483_648_u64,
                })
            })
            .collect();
        // Always include one LXC container to verify the plugin ignores LXC containers.
        items.push(json!({
            "type": "lxc",
            "vmid": 999,
            "node": "pve1",
            "name": "lxc-ignored",
            "status": "running",
        }));
        return (200, json!({ "data": items }), Vec::new());
    }

    let segments: Vec<&str> = req.path.trim_start_matches('/').split('/').collect();
    if segments.len() >= 5
        && segments[0] == "api2"
        && segments[1] == "json"
        && segments[2] == "nodes"
    {
        let node = segments[3];
        if segments[4] == "qemu" && segments.len() >= 6 {
            let Ok(vmid) = segments[5].parse::<u32>() else {
                return (404, json!({ "data": null }), Vec::new());
            };

            if req.method == "GET" && segments.get(6) == Some(&"config") {
                return match guard.vms.iter().find(|v| v.vmid == vmid && v.node == node) {
                    Some(vm) => (200, json!({ "data": vm.config }), Vec::new()),
                    None => (404, json!({ "data": null }), Vec::new()),
                };
            }

            if (req.method == "POST" || req.method == "PUT") && segments.get(6) == Some(&"config") {
                let form: BTreeMap<String, String> = url::form_urlencoded::parse(&req.body)
                    .into_owned()
                    .collect();
                let Some(vm) = guard
                    .vms
                    .iter_mut()
                    .find(|v| v.vmid == vmid && v.node == node)
                else {
                    return (404, json!({ "data": null }), Vec::new());
                };
                for (k, v) in &form {
                    vm.config.insert(k.clone(), v.clone());
                }
                guard.config_updates.push((vmid, form));
                return (200, json!({ "data": null }), Vec::new());
            }

            if req.method == "GET"
                && segments.get(6) == Some(&"status")
                && segments.get(7) == Some(&"current")
            {
                return match guard.vms.iter().find(|v| v.vmid == vmid && v.node == node) {
                    Some(vm) => (
                        200,
                        json!({
                            "data": {
                                "status": vm.status,
                                "name": vm.name,
                                "cpus": 2,
                                "maxmem": 2_147_483_648_u64,
                            }
                        }),
                        Vec::new(),
                    ),
                    None => (404, json!({ "data": null }), Vec::new()),
                };
            }

            if req.method == "POST" && segments.get(6) == Some(&"status") {
                let action = segments.get(7).copied().unwrap_or_default();
                let Some(vm) = guard
                    .vms
                    .iter_mut()
                    .find(|v| v.vmid == vmid && v.node == node)
                else {
                    return (404, json!({ "data": null }), Vec::new());
                };
                match action {
                    "start" | "reboot" | "reset" => "running".clone_into(&mut vm.status),
                    "stop" | "shutdown" => "stopped".clone_into(&mut vm.status),
                    _ => return (404, json!({ "data": null }), Vec::new()),
                }
                guard.power_actions.push((vmid, action.to_owned()));
                let upid = format!("UPID:{node}:0001:0001:0001:qm{action}:{vmid}:lab@pve:");
                return (200, json!({ "data": upid }), Vec::new());
            }

            if req.method == "POST" && segments.get(6) == Some(&"vncproxy") {
                if !guard.vms.iter().any(|v| v.vmid == vmid && v.node == node) {
                    return (404, json!({ "data": null }), Vec::new());
                }
                return (
                    200,
                    json!({
                        "data": {
                            "ticket": VNC_TICKET,
                            "port": "5900",
                            "user": USERNAME,
                            "upid": format!("UPID:{node}:0002:0002:0002:vncproxy:{vmid}:lab@pve:"),
                        }
                    }),
                    guard.vncproxy_set_cookies.clone(),
                );
            }
        }

        if segments[4] == "storage" && req.method == "GET" {
            if segments.len() == 5 {
                let list: Vec<Value> = guard
                    .isos
                    .keys()
                    .map(|storage| json!({ "storage": storage, "content": "iso,vztmpl" }))
                    .collect();
                return (200, json!({ "data": list }), Vec::new());
            }
            if segments.len() == 7 && segments[6] == "content" {
                let storage = segments[5];
                let list: Vec<Value> = guard
                    .isos
                    .get(storage)
                    .into_iter()
                    .flatten()
                    .map(|(volid, size)| json!({ "volid": volid, "size": size, "content": "iso" }))
                    .collect();
                return (200, json!({ "data": list }), Vec::new());
            }
        }

        if segments[4] == "tasks" && req.method == "GET" && segments.get(6) == Some(&"status") {
            return (
                200,
                json!({ "data": { "status": "stopped", "exitstatus": "OK" } }),
                Vec::new(),
            );
        }
    }

    (404, json!({ "data": null }), Vec::new())
}

struct VncSocketGuard(Arc<AtomicUsize>);

impl Drop for VncSocketGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "fake Proxmox WebSocket + RFB 3.8 server loop in test support"
)]
async fn handle_vnc_websocket<S>(
    mut stream: S,
    req: HttpRequest,
    negotiated_alpn: Option<Vec<u8>>,
    state: Arc<Mutex<FakeState>>,
    active_vnc_sockets: Arc<AtomicUsize>,
) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (required_cookies, custom_error) = {
        let mut guard = state.lock().unwrap();
        guard.last_ws_headers = Some(req.headers.clone());
        guard.last_ws_alpn = negotiated_alpn;
        (
            guard.require_ws_cookies.clone(),
            guard.ws_error_response.clone(),
        )
    };

    if let Some(raw_error) = custom_error {
        stream.write_all(raw_error.as_bytes()).await?;
        stream.flush().await?;
        return Ok(());
    }

    let cookie_header = req.header("cookie").unwrap_or_default();
    let proxy_cookies_ok = required_cookies
        .iter()
        .all(|pair| cookie_header.contains(pair));
    if !proxy_cookies_ok {
        let bad_gateway = "HTTP/1.0 502 Bad Gateway\r\nServer: haproxy\r\n\r\n\
            <html><body><h1>502 Bad Gateway</h1>\nMissing sticky proxy cookie.\n</body></html>";
        stream.write_all(bad_gateway.as_bytes()).await?;
        stream.flush().await?;
        return Ok(());
    }

    let params: BTreeMap<String, String> = url::form_urlencoded::parse(req.query.as_bytes())
        .into_owned()
        .collect();
    let vnc_ok =
        req.has_valid_cookie() && params.get("vncticket").map(String::as_str) == Some(VNC_TICKET);
    if !vnc_ok {
        write_http_json(&mut stream, 401, &json!({ "data": null }), &[]).await?;
        return Ok(());
    }

    let segments: Vec<&str> = req.path.trim_start_matches('/').split('/').collect();
    let vmid = segments
        .get(5)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(100);

    let key = req.header("sec-websocket-key").unwrap_or_default();
    let mut accept_input = String::from(key);
    accept_input.push_str("258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    let accept_hash = digest(&SHA1_FOR_LEGACY_USE_ONLY, accept_input.as_bytes());
    let accept_val = base64::engine::general_purpose::STANDARD.encode(accept_hash.as_ref());

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept_val}\r\nSec-WebSocket-Protocol: binary\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;

    active_vnc_sockets.fetch_add(1, Ordering::SeqCst);
    let _guard = VncSocketGuard(active_vnc_sockets);

    // 1. RFB 3.8 handshake
    ws_send_binary(&mut stream, b"RFB 003.008\n").await?;
    let mut rfb_buf = Vec::new();
    let client_ver = ws_read_exact(&mut stream, &mut rfb_buf, 12).await?;
    if !client_ver.starts_with(b"RFB 003.") {
        return Ok(());
    }

    // Security types: 1 type = VNC auth (2)
    ws_send_binary(&mut stream, &[1, 2]).await?;
    let sec_choice = ws_read_exact(&mut stream, &mut rfb_buf, 1).await?;
    if sec_choice != [2] {
        return Ok(());
    }

    // 16-byte challenge -> accept 16-byte DES response
    let challenge = [0x11u8; 16];
    ws_send_binary(&mut stream, &challenge).await?;
    let _des_response = ws_read_exact(&mut stream, &mut rfb_buf, 16).await?;
    // SecurityResult = OK (0)
    ws_send_binary(&mut stream, &0u32.to_be_bytes()).await?;

    // ClientInit (1 byte)
    let _client_init = ws_read_exact(&mut stream, &mut rfb_buf, 1).await?;

    // ServerInit
    let (width, height, name) = {
        let guard = state.lock().unwrap();
        let vm = guard.vms.iter().find(|v| v.vmid == vmid).unwrap();
        (vm.width, vm.height, vm.name.clone())
    };
    let mut server_init = Vec::new();
    server_init.extend_from_slice(&width.to_be_bytes());
    server_init.extend_from_slice(&height.to_be_bytes());
    // 16-byte PixelFormat: 32bpp, depth 24, little-endian, true-color, R/G/B max 255, shifts 0/8/16
    server_init.extend_from_slice(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 0, 8, 16, 0, 0, 0]);
    server_init.extend_from_slice(&u32::try_from(name.len()).unwrap().to_be_bytes());
    server_init.extend_from_slice(name.as_bytes());
    ws_send_binary(&mut stream, &server_init).await?;

    // 2. RFB client message loop
    let mut seq_tick: u8 = 1;
    loop {
        let Ok(msg_type) = ws_read_exact(&mut stream, &mut rfb_buf, 1).await else {
            return Ok(());
        };
        match msg_type[0] {
            0 => {
                // SetPixelFormat
                drop(ws_read_exact(&mut stream, &mut rfb_buf, 19).await?);
            }
            2 => {
                // SetEncodings
                let hdr = ws_read_exact(&mut stream, &mut rfb_buf, 3).await?;
                let count = usize::from(u16::from_be_bytes([hdr[1], hdr[2]]));
                drop(ws_read_exact(&mut stream, &mut rfb_buf, count * 4).await?);
            }
            3 => {
                // FramebufferUpdateRequest: reply immediately on non-incremental (full)
                // requests; incremental requests wait for actual screen changes.
                let body = ws_read_exact(&mut stream, &mut rfb_buf, 9).await?;
                if body[0] == 0 {
                    let frame = encode_rfb_update(&state, vmid);
                    ws_send_binary(&mut stream, &frame).await?;
                }
            }
            4 => {
                // KeyEvent
                let payload = ws_read_exact(&mut stream, &mut rfb_buf, 7).await?;
                let down = payload[0] != 0;
                let keysym = u32::from_be_bytes([payload[3], payload[4], payload[5], payload[6]]);
                {
                    let mut guard = state.lock().unwrap();
                    guard.vnc_key_events.push((vmid, keysym, down));

                    if let Some(vm) = guard.vms.iter_mut().find(|v| v.vmid == vmid) {
                        seq_tick = seq_tick.wrapping_add(17);
                        vm.pixels[0] = seq_tick;
                        vm.pixels[1] = seq_tick.wrapping_add(50);
                    }

                    guard.vnc_key_event_received.notify_waiters();
                }
                let frame = encode_rfb_update(&state, vmid);
                ws_send_binary(&mut stream, &frame).await?;
            }
            5 => {
                // PointerEvent
                let payload = ws_read_exact(&mut stream, &mut rfb_buf, 5).await?;
                let mask = payload[0];
                let x = u16::from_be_bytes([payload[1], payload[2]]);
                let y = u16::from_be_bytes([payload[3], payload[4]]);
                {
                    let mut guard = state.lock().unwrap();
                    guard.vnc_pointer_events.push((vmid, x, y, mask));
                    if let Some(vm) = guard.vms.iter_mut().find(|v| v.vmid == vmid) {
                        seq_tick = seq_tick.wrapping_add(29);
                        vm.pixels[4] = seq_tick;
                        vm.pixels[5] = seq_tick.wrapping_add(80);
                    }
                }
                let frame = encode_rfb_update(&state, vmid);
                ws_send_binary(&mut stream, &frame).await?;
            }
            6 => {
                // ClientCutText
                let hdr = ws_read_exact(&mut stream, &mut rfb_buf, 7).await?;
                let len = u32::from_be_bytes([hdr[3], hdr[4], hdr[5], hdr[6]]) as usize;
                drop(ws_read_exact(&mut stream, &mut rfb_buf, len).await?);
            }
            _ => return Ok(()),
        }
    }
}

fn encode_rfb_update(state: &Arc<Mutex<FakeState>>, vmid: u32) -> Vec<u8> {
    let guard = state.lock().unwrap();
    let vm = guard.vms.iter().find(|v| v.vmid == vmid).unwrap();
    let mut out = Vec::new();
    match vm.rfb_mode {
        RfbMode::Raw => {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&0u16.to_be_bytes());
            out.extend_from_slice(&0u16.to_be_bytes());
            out.extend_from_slice(&vm.width.to_be_bytes());
            out.extend_from_slice(&vm.height.to_be_bytes());
            out.extend_from_slice(&0i32.to_be_bytes()); // Raw
            out.extend_from_slice(&vm.pixels);
        }
        RfbMode::Zlib => {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
            enc.write_all(&vm.pixels).unwrap();
            let compressed = enc.finish().unwrap();
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&0u16.to_be_bytes());
            out.extend_from_slice(&0u16.to_be_bytes());
            out.extend_from_slice(&vm.width.to_be_bytes());
            out.extend_from_slice(&vm.height.to_be_bytes());
            out.extend_from_slice(&6i32.to_be_bytes()); // Zlib
            out.extend_from_slice(&u32::try_from(compressed.len()).unwrap().to_be_bytes());
            out.extend_from_slice(&compressed);
        }
        RfbMode::CopyRect => {
            // Send a Raw full frame followed by a CopyRect block.
            out.extend_from_slice(&[0, 0, 0, 2]);
            out.extend_from_slice(&0u16.to_be_bytes());
            out.extend_from_slice(&0u16.to_be_bytes());
            out.extend_from_slice(&vm.width.to_be_bytes());
            out.extend_from_slice(&vm.height.to_be_bytes());
            out.extend_from_slice(&0i32.to_be_bytes());
            out.extend_from_slice(&vm.pixels);
            // CopyRect 8x8 from (0,0) to (8,8)
            out.extend_from_slice(&8u16.to_be_bytes());
            out.extend_from_slice(&8u16.to_be_bytes());
            out.extend_from_slice(&8u16.to_be_bytes());
            out.extend_from_slice(&8u16.to_be_bytes());
            out.extend_from_slice(&1i32.to_be_bytes());
            out.extend_from_slice(&0u16.to_be_bytes());
            out.extend_from_slice(&0u16.to_be_bytes());
        }
    }
    out
}

async fn ws_send_binary<S: AsyncWrite + Unpin>(
    stream: &mut S,
    payload: &[u8],
) -> std::io::Result<()> {
    let mut header = Vec::with_capacity(10);
    header.push(0x82); // FIN + Binary
    if payload.len() < 126 {
        header.push(u8::try_from(payload.len()).unwrap());
    } else if let Ok(len16) = u16::try_from(payload.len()) {
        header.push(126);
        header.extend_from_slice(&len16.to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    stream.write_all(&header).await?;
    stream.write_all(payload).await?;
    stream.flush().await
}

async fn ws_read_exact<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    buf: &mut Vec<u8>,
    count: usize,
) -> std::io::Result<Vec<u8>> {
    while buf.len() < count {
        let mut hdr = [0u8; 2];
        stream.read_exact(&mut hdr).await?;
        let opcode = hdr[0] & 0x0f;
        let masked = (hdr[1] & 0x80) != 0;
        let len_code = hdr[1] & 0x7f;
        let payload_len = match len_code {
            0..=125 => usize::from(len_code),
            126 => {
                let mut ext = [0u8; 2];
                stream.read_exact(&mut ext).await?;
                usize::from(u16::from_be_bytes(ext))
            }
            127 => {
                let mut ext = [0u8; 8];
                stream.read_exact(&mut ext).await?;
                usize::try_from(u64::from_be_bytes(ext)).unwrap()
            }
            _ => unreachable!(),
        };
        let mask_key = if masked {
            let mut key = [0u8; 4];
            stream.read_exact(&mut key).await?;
            Some(key)
        } else {
            None
        };
        let mut payload = vec![0u8; payload_len];
        stream.read_exact(&mut payload).await?;
        if let Some(key) = mask_key {
            for (idx, b) in payload.iter_mut().enumerate() {
                *b ^= key[idx % 4];
            }
        }
        match opcode {
            0x0..=0x2 => buf.extend_from_slice(&payload),
            0x8 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "ws closed",
                ));
            }
            0x9 => {
                // Ping -> Pong
                let mut pong = vec![0x8a, u8::try_from(payload.len()).unwrap()];
                pong.extend_from_slice(&payload);
                stream.write_all(&pong).await?;
                stream.flush().await?;
            }
            _ => {}
        }
    }
    Ok(buf.drain(..count).collect())
}
