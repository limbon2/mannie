//! Proxmox VE HTTPS REST API client (ADR-0080):
//! - Uses a shared `rustls::ClientConfig` (OS trust or pinned leaf SHA-256)
//!   via `reqwest::ClientBuilder::tls_backend_preconfigured`.
//! - Disables automatic redirects (`Policy::none()`) so auth cookies never
//!   cross origins.
//! - Manages per-server `PVEAuthCookie` + `CSRFPreventionToken` in memory with
//!   automatic renewal before 90 minutes and single retry on 401.
//! - Rejects TFA-protected accounts with `ProxmoxError::TfaUnsupported`.
//! - Scrubs secrets from all error messages.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_plugin_sdk::contracts::Secret;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::model::{ProxmoxError, ServerRecord, is_valid_node};
use crate::tls::{build_tls_config, classify_error_chain};

/// Renew Proxmox tickets proactively at 90 minutes (Proxmox tickets expire at 2 hours).
const TICKET_RENEW_AFTER: Duration = Duration::from_mins(90);

/// Default REST request timeout.
const REST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum response payload read from Proxmox REST endpoints (4 MiB).
const MAX_API_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Poll interval while waiting for a Proxmox task UPID to finish.
const TASK_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Maximum wait for a power action task when `wait = true`.
const TASK_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

/// QEMU VM summary returned by `GET /api2/json/cluster/resources?type=vm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProxmoxVmInfo {
    pub(crate) vmid: u32,
    pub(crate) node: String,
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) maxcpu: Option<u32>,
    pub(crate) maxmem: Option<u64>,
}

/// ISO image entry returned by `GET /api2/json/nodes/{node}/storage/{storage}/content?content=iso`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProxmoxIsoInfo {
    pub(crate) storage: String,
    pub(crate) volid: String,
    pub(crate) size: Option<u64>,
}

/// Result of listing ISO images on a Proxmox node, distinguishing storages
/// that were inspected from storages hidden or denied by permissions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IsoListResult {
    pub(crate) isos: Vec<ProxmoxIsoInfo>,
    pub(crate) checked_storages: Vec<String>,
    pub(crate) denied_storages: Vec<String>,
}

/// Short-lived VNC WebSocket proxy ticket returned by `POST .../vncproxy`.
#[derive(Debug)]
pub(crate) struct VncProxyTicket {
    pub(crate) port: u16,
    pub(crate) ticket: Zeroizing<String>,
    pub(crate) password: Zeroizing<String>,
}

/// Cached Proxmox authentication ticket kept in Rust memory only.
#[derive(Debug)]
struct CachedTicket {
    ticket: Zeroizing<String>,
    csrf: Zeroizing<String>,
    obtained_at: Instant,
}

/// Per-server Proxmox VE REST API client with shared TLS verification and
/// in-memory ticket cache.
#[derive(Debug)]
pub(crate) struct ProxmoxApiClient {
    endpoint: String,
    username: String,
    http: reqwest::Client,
    ticket: Mutex<Option<CachedTicket>>,
}

impl ProxmoxApiClient {
    /// Builds a REST client bound to `server`'s canonical endpoint and TLS trust.
    pub(crate) fn new(server: &ServerRecord) -> Result<Self, ProxmoxError> {
        let tls_config = build_tls_config(
            server.pinned_cert_sha256.as_deref(),
            vec![b"http/1.1".to_vec()],
        )?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("Mannie/", env!("CARGO_PKG_VERSION")))
            .tls_backend_preconfigured(tls_config)
            // Connect directly to the Proxmox host just like certificate
            // inspection and VNC WebSockets (`connect_tls`), and prevent Cargo
            // feature unification (`reqwest/system-proxy` from sibling crates)
            // from hijacking LAN/Hyper-V hosts into a system proxy when
            // `NO_PROXY` suppresses Windows `ProxyOverride` rules.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .pool_max_idle_per_host(0)
            .connect_timeout(Duration::from_secs(15))
            .timeout(REST_TIMEOUT)
            .build()
            .map_err(|error| ProxmoxError::ConnectionFailed {
                detail: format!("could not build HTTPS client: {error}"),
            })?;

        Ok(Self {
            endpoint: server.endpoint(),
            username: server.username.clone(),
            http,
            ticket: Mutex::new(None),
        })
    }

    /// Clears the cached ticket from memory immediately.
    pub(crate) async fn clear_ticket(&self) {
        let mut guard = self.ticket.lock().await;
        *guard = None;
    }

    /// Authenticates (or renews) and returns the current `PVEAuthCookie` value
    /// for opening a `vncwebsocket` connection.
    pub(crate) async fn current_cookie_ticket(
        &self,
        password: &Secret,
    ) -> Result<Zeroizing<String>, ProxmoxError> {
        let (ticket, _csrf) = self.ensure_ticket(password, false).await?;
        Ok(ticket)
    }

    /// Lists all non-template QEMU VMs visible to the configured Proxmox user.
    pub(crate) async fn list_qemu_vms(
        &self,
        password: &Secret,
    ) -> Result<Vec<ProxmoxVmInfo>, ProxmoxError> {
        let data = self
            .request_json(
                reqwest::Method::GET,
                "/api2/json/cluster/resources?type=vm",
                None,
                password,
            )
            .await?;
        let items: Vec<ClusterResource> =
            serde_json::from_value(data).map_err(|error| ProxmoxError::ApiError {
                detail: format!("unexpected cluster/resources shape: {error}"),
            })?;

        let mut vms = Vec::new();
        for item in items {
            let is_qemu = item.resource_type.as_deref() == Some("qemu");
            let is_template = item.template.unwrap_or(0) != 0;
            if !is_qemu || is_template {
                continue;
            }

            let Some(vmid) = item.vmid.filter(|&id| id > 0) else {
                continue;
            };
            let Some(node) = item.node.filter(|n| is_valid_node(n)) else {
                continue;
            };
            let name = item
                .name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| format!("VM {vmid}"));
            let status = item.status.unwrap_or_else(|| "unknown".to_owned());
            vms.push(ProxmoxVmInfo {
                vmid,
                node,
                name,
                status,
                maxcpu: item.maxcpu,
                maxmem: item.maxmem,
            });
        }

        vms.sort_by_key(|vm| vm.vmid);
        Ok(vms)
    }

    /// Reads current runtime status for a single QEMU VM.
    pub(crate) async fn get_vm_status(
        &self,
        node: &str,
        vmid: u32,
        password: &Secret,
    ) -> Result<Value, ProxmoxError> {
        let path = format!("/api2/json/nodes/{node}/qemu/{vmid}/status/current");
        self.request_json(reqwest::Method::GET, &path, None, password)
            .await
    }

    /// Reads current hardware and boot configuration for a single QEMU VM.
    pub(crate) async fn get_vm_config(
        &self,
        node: &str,
        vmid: u32,
        password: &Secret,
    ) -> Result<BTreeMap<String, Value>, ProxmoxError> {
        let path = format!("/api2/json/nodes/{node}/qemu/{vmid}/config");
        let data = self
            .request_json(reqwest::Method::GET, &path, None, password)
            .await?;
        serde_json::from_value(data).map_err(|error| ProxmoxError::ApiError {
            detail: format!("unexpected qemu config shape: {error}"),
        })
    }

    /// Lists ISO images available on `node`, optionally restricted to `storage_filter`.
    pub(crate) async fn list_isos(
        &self,
        node: &str,
        storage_filter: Option<&str>,
        password: &Secret,
    ) -> Result<IsoListResult, ProxmoxError> {
        let storages: Vec<String> = if let Some(storage) = storage_filter {
            vec![storage.to_owned()]
        } else {
            let path = format!("/api2/json/nodes/{node}/storage");
            let data = self
                .request_json(reqwest::Method::GET, &path, None, password)
                .await?;
            let entries: Vec<NodeStorage> =
                serde_json::from_value(data).map_err(|error| ProxmoxError::ApiError {
                    detail: format!("unexpected storage list shape: {error}"),
                })?;
            entries
                .into_iter()
                .filter(|entry| {
                    let enabled = entry.enabled.unwrap_or(1) != 0 && entry.active.unwrap_or(1) != 0;
                    let holds_iso = entry
                        .content
                        .as_deref()
                        .is_some_and(|c| c.split(',').any(|kind| kind.trim() == "iso"));
                    enabled && holds_iso
                })
                .map(|entry| entry.storage)
                .collect()
        };

        let mut isos = Vec::new();
        let mut checked_storages = Vec::new();
        let mut denied_storages = Vec::new();
        for storage in storages {
            let encoded_storage = encode_path_segment(&storage);
            let path =
                format!("/api2/json/nodes/{node}/storage/{encoded_storage}/content?content=iso");
            let data = match self
                .request_json(reqwest::Method::GET, &path, None, password)
                .await
            {
                Ok(data) => data,
                Err(ProxmoxError::PermissionDenied { .. }) if storage_filter.is_none() => {
                    // Record denied storages instead of silently dropping them so callers
                    // can distinguish permission gaps from genuinely empty ISO storages.
                    denied_storages.push(storage);
                    continue;
                }
                Err(error) => return Err(error),
            };
            checked_storages.push(storage.clone());
            let items: Vec<StorageContent> =
                serde_json::from_value(data).map_err(|error| ProxmoxError::ApiError {
                    detail: format!("unexpected storage content shape: {error}"),
                })?;
            for item in items {
                if item.content.as_deref() == Some("iso") || item.volid.contains(":iso/") {
                    isos.push(ProxmoxIsoInfo {
                        storage: storage.clone(),
                        volid: item.volid,
                        size: item.size,
                    });
                }
            }
        }

        isos.sort_by(|left, right| left.volid.cmp(&right.volid));
        checked_storages.sort();
        denied_storages.sort();
        Ok(IsoListResult {
            isos,
            checked_storages,
            denied_storages,
        })
    }

    /// Applies key/value QEMU config parameters via `PUT .../config`.
    pub(crate) async fn update_vm_config(
        &self,
        node: &str,
        vmid: u32,
        params: &[(&str, String)],
        password: &Secret,
    ) -> Result<Value, ProxmoxError> {
        let path = format!("/api2/json/nodes/{node}/qemu/{vmid}/config");
        self.request_json(reqwest::Method::PUT, &path, Some(params), password)
            .await
    }

    /// Triggers a QEMU VM power action (`start`, `shutdown`, `reboot`, `stop`, `reset`)
    /// and returns the Proxmox task UPID string if present.
    pub(crate) async fn vm_power(
        &self,
        node: &str,
        vmid: u32,
        action: &str,
        password: &Secret,
    ) -> Result<Option<String>, ProxmoxError> {
        let path = format!("/api2/json/nodes/{node}/qemu/{vmid}/status/{action}");
        let data = self
            .request_json(reqwest::Method::POST, &path, Some(&[]), password)
            .await?;
        Ok(data.as_str().map(str::to_owned))
    }

    /// Polls a Proxmox task UPID until it stops or `TASK_WAIT_TIMEOUT` elapses.
    pub(crate) async fn wait_task(
        &self,
        node: &str,
        upid: &str,
        password: &Secret,
    ) -> Result<String, ProxmoxError> {
        let encoded = encode_path_segment(upid);
        let path = format!("/api2/json/nodes/{node}/tasks/{encoded}/status");
        let deadline = Instant::now() + TASK_WAIT_TIMEOUT;

        loop {
            let data = self
                .request_json(reqwest::Method::GET, &path, None, password)
                .await?;
            let status = data
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if status == "stopped" {
                let exit = data
                    .get("exitstatus")
                    .and_then(Value::as_str)
                    .unwrap_or("OK");
                return Ok(exit.to_owned());
            }

            if Instant::now() >= deadline {
                return Err(ProxmoxError::Timeout);
            }

            tokio::time::sleep(TASK_POLL_INTERVAL).await;
        }
    }

    /// Calls `POST .../vncproxy` with `websocket=1&generate-password=1` to
    /// obtain a short-lived WebSocket port, `vncticket`, and RFB password.
    pub(crate) async fn create_vnc_proxy(
        &self,
        node: &str,
        vmid: u32,
        password: &Secret,
    ) -> Result<VncProxyTicket, ProxmoxError> {
        let path = format!("/api2/json/nodes/{node}/qemu/{vmid}/vncproxy");
        let params = [
            ("websocket", "1".to_owned()),
            ("generate-password", "1".to_owned()),
        ];
        let data = self
            .request_json(reqwest::Method::POST, &path, Some(&params), password)
            .await?;
        let parsed: VncProxyResponse =
            serde_json::from_value(data).map_err(|error| ProxmoxError::ApiError {
                detail: format!("unexpected vncproxy response shape: {error}"),
            })?;
        let port = parsed.port.as_u16().ok_or_else(|| ProxmoxError::ApiError {
            detail: "vncproxy response contained an invalid port".to_owned(),
        })?;

        Ok(VncProxyTicket {
            port,
            ticket: Zeroizing::new(parsed.ticket),
            password: Zeroizing::new(parsed.password.unwrap_or_default()),
        })
    }

    /// Sends one authenticated Proxmox JSON API request, retrying once with a
    /// fresh ticket on HTTP 401.
    async fn request_json(
        &self,
        method: reqwest::Method,
        path_and_query: &str,
        form_params: Option<&[(&str, String)]>,
        password: &Secret,
    ) -> Result<Value, ProxmoxError> {
        let mut force_login = false;

        for attempt in 0..2 {
            let (ticket, csrf) = self.ensure_ticket(password, force_login).await?;
            let url = format!("{}{path_and_query}", self.endpoint);
            let mut headers = HeaderMap::new();
            let cookie_val = HeaderValue::from_str(&format!("PVEAuthCookie={}", ticket.as_str()))
                .map_err(|_err| ProxmoxError::ApiError {
                detail: "invalid Proxmox auth cookie header".to_owned(),
            })?;
            headers.insert(COOKIE, cookie_val);

            let mutates = method != reqwest::Method::GET && method != reqwest::Method::HEAD;
            if mutates {
                let csrf_val = HeaderValue::from_str(csrf.as_str()).map_err(|_err| {
                    ProxmoxError::ApiError {
                        detail: "invalid Proxmox CSRF header".to_owned(),
                    }
                })?;
                headers.insert("CSRFPreventionToken", csrf_val);
            }

            let mut req = self.http.request(method.clone(), &url).headers(headers);
            if let Some(params) = form_params {
                let body = encode_form(params);
                req = req
                    .header(
                        CONTENT_TYPE,
                        HeaderValue::from_static("application/x-www-form-urlencoded"),
                    )
                    .body(body);
            }

            let response = req
                .send()
                .await
                .map_err(|error| map_reqwest_error(&error, password.expose(), &ticket, &csrf))?;
            let status = response.status();

            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                force_login = true;
                continue;
            }

            let bytes = read_bounded_body(response, password.expose(), &ticket, &csrf).await?;
            return parse_api_response(status, &bytes, password.expose(), &ticket, &csrf);
        }

        Err(ProxmoxError::AuthenticationFailed)
    }

    /// Ensures a valid cached Proxmox ticket exists, renewing proactively or
    /// authenticating with username/password.
    async fn ensure_ticket(
        &self,
        password: &Secret,
        force_reauth: bool,
    ) -> Result<(Zeroizing<String>, Zeroizing<String>), ProxmoxError> {
        let mut guard = self.ticket.lock().await;

        if !force_reauth
            && let Some(cached) = guard.as_ref()
            && cached.obtained_at.elapsed() < TICKET_RENEW_AFTER
        {
            return Ok((cached.ticket.clone(), cached.csrf.clone()));
        }

        // Try renewing with the existing ticket first when not forced to reauth.
        if !force_reauth
            && let Some(existing) = guard.as_ref()
            && let Ok(renewed) = self.fetch_ticket(existing.ticket.as_str(), password).await
        {
            let pair = (renewed.ticket.clone(), renewed.csrf.clone());
            *guard = Some(renewed);
            return Ok(pair);
        }

        let fresh = self.fetch_ticket(password.expose(), password).await?;
        let pair = (fresh.ticket.clone(), fresh.csrf.clone());
        *guard = Some(fresh);
        Ok(pair)
    }

    async fn fetch_ticket(
        &self,
        credential: &str,
        password_for_scrub: &Secret,
    ) -> Result<CachedTicket, ProxmoxError> {
        let url = format!("{}/api2/json/access/ticket", self.endpoint);
        let body = encode_form(&[
            ("username", self.username.clone()),
            ("password", credential.to_owned()),
        ]);

        let response = self
            .http
            .post(&url)
            .header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/x-www-form-urlencoded"),
            )
            .body(body)
            .send()
            .await
            .map_err(|error| {
                map_reqwest_error(&error, password_for_scrub.expose(), credential, "")
            })?;
        let status = response.status();
        let bytes =
            read_bounded_body(response, password_for_scrub.expose(), credential, "").await?;

        if status == StatusCode::UNAUTHORIZED {
            return Err(ProxmoxError::AuthenticationFailed);
        }

        let data = parse_api_response(status, &bytes, password_for_scrub.expose(), credential, "")?;
        let parsed: TicketResponse =
            serde_json::from_value(data).map_err(|error| ProxmoxError::ApiError {
                detail: format!("unexpected access/ticket response: {error}"),
            })?;

        let needs_tfa = parsed.need_tfa.unwrap_or(0) != 0
            || parsed.ticket.starts_with("PVE:!tfa!")
            || parsed.csrf.as_deref().unwrap_or_default().is_empty();
        if needs_tfa {
            return Err(ProxmoxError::TfaUnsupported);
        }

        let csrf = parsed.csrf.ok_or(ProxmoxError::TfaUnsupported)?;
        Ok(CachedTicket {
            ticket: Zeroizing::new(parsed.ticket),
            csrf: Zeroizing::new(csrf),
            obtained_at: Instant::now(),
        })
    }
}

/// Scrubs any known secret string from diagnostic error details so no password,
/// ticket, or CSRF token can leak into logs or tool results.
pub(crate) fn scrub_secrets(text: &str, secrets: &[&str]) -> String {
    let mut cleaned = text.to_owned();
    for secret in secrets {
        if secret.len() >= 4 {
            cleaned = cleaned.replace(secret, "[redacted]");
        }
    }
    cleaned
}

fn encode_form(params: &[(&str, String)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in params {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

fn encode_path_segment(segment: &str) -> String {
    url::form_urlencoded::byte_serialize(segment.as_bytes()).collect()
}

async fn read_bounded_body(
    mut response: reqwest::Response,
    password: &str,
    ticket: &str,
    csrf: &str,
) -> Result<Vec<u8>, ProxmoxError> {
    let mut out = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| map_reqwest_error(&error, password, ticket, csrf))?
    {
        if out.len().saturating_add(chunk.len()) > MAX_API_RESPONSE_BYTES {
            return Err(ProxmoxError::ApiError {
                detail: "Proxmox API response exceeded size limit".to_owned(),
            });
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn parse_api_response(
    status: StatusCode,
    bytes: &[u8],
    password: &str,
    ticket: &str,
    csrf: &str,
) -> Result<Value, ProxmoxError> {
    if status == StatusCode::UNAUTHORIZED {
        return Err(ProxmoxError::AuthenticationFailed);
    }

    if status == StatusCode::FORBIDDEN {
        let detail = extract_error_detail(status, bytes, &[password, ticket, csrf]);
        return Err(ProxmoxError::PermissionDenied { detail });
    }

    if status == StatusCode::NOT_FOUND {
        return Err(ProxmoxError::VmNotFound);
    }

    if !status.is_success() {
        let detail = extract_error_detail(status, bytes, &[password, ticket, csrf]);
        return Err(ProxmoxError::ApiError { detail });
    }

    let envelope: ApiEnvelope =
        serde_json::from_slice(bytes).map_err(|error| ProxmoxError::ApiError {
            detail: format!("invalid JSON from Proxmox API: {error}"),
        })?;
    Ok(envelope.data)
}

fn extract_error_detail(status: StatusCode, bytes: &[u8], secrets: &[&str]) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes)
        && let Some(msg) = value.get("message").and_then(Value::as_str)
    {
        let clean = scrub_secrets(msg.trim(), secrets);
        if !clean.is_empty() {
            return format!("HTTP {}: {clean}", status.as_u16());
        }
    }

    let body = String::from_utf8_lossy(bytes);
    let snippet: String = body.trim().chars().take(240).collect();
    let clean = scrub_secrets(&snippet, secrets);
    if clean.is_empty() {
        format!("HTTP {}", status.as_u16())
    } else {
        format!("HTTP {}: {clean}", status.as_u16())
    }
}

fn map_reqwest_error(
    error: &reqwest::Error,
    password: &str,
    ticket: &str,
    csrf: &str,
) -> ProxmoxError {
    if error.is_timeout() {
        return ProxmoxError::Timeout;
    }

    if let Some(tls_err) = classify_error_chain(error) {
        return tls_err;
    }

    let detail = scrub_secrets(&error.to_string(), &[password, ticket, csrf]);
    ProxmoxError::ConnectionFailed { detail }
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope {
    #[serde(default)]
    data: Value,
}

#[derive(Debug, Deserialize)]
struct TicketResponse {
    ticket: String,
    #[serde(rename = "CSRFPreventionToken")]
    csrf: Option<String>,
    #[serde(rename = "NeedTFA")]
    need_tfa: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct ClusterResource {
    #[serde(rename = "type")]
    resource_type: Option<String>,
    vmid: Option<u32>,
    node: Option<String>,
    name: Option<String>,
    status: Option<String>,
    template: Option<u8>,
    maxcpu: Option<u32>,
    maxmem: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct NodeStorage {
    storage: String,
    content: Option<String>,
    enabled: Option<u8>,
    active: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct StorageContent {
    volid: String,
    content: Option<String>,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct VncProxyResponse {
    port: PortValue,
    ticket: String,
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PortValue {
    Number(u16),
    Text(String),
}

impl PortValue {
    fn as_u16(&self) -> Option<u16> {
        match self {
            Self::Number(port) => (*port > 0).then_some(*port),
            Self::Text(text) => text.parse::<u16>().ok().filter(|&p| p > 0),
        }
    }
}

/// Percent-encodes a `vncticket` for use in `vncwebsocket` query parameters.
pub(crate) fn encode_vnc_ticket(ticket: &str) -> String {
    url::form_urlencoded::byte_serialize(ticket.as_bytes()).collect()
}

/// Shared TLS config wrapper helper for opening VNC WebSocket TLS connections.
pub(crate) fn build_vnc_tls_config(
    server: &ServerRecord,
) -> Result<Arc<rustls::ClientConfig>, ProxmoxError> {
    build_tls_config(server.pinned_cert_sha256.as_deref(), Vec::new()).map(Arc::new)
}
