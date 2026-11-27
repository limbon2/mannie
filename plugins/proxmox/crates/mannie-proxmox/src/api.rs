//! Proxmox VE HTTPS REST API client (ADR-0080):
//! - Uses a shared `rustls::ClientConfig` (OS trust or pinned leaf SHA-256)
//!   via `reqwest::ClientBuilder::tls_backend_preconfigured`.
//! - Allows only HTTPS requests.
//! - Disables automatic redirects (`Policy::none()`) so auth cookies never
//!   cross origins.
//! - Manages per-server `PVEAuthCookie` + `CSRFPreventionToken` in memory with
//!   automatic renewal before 90 minutes and single retry on 401.
//! - Rejects TFA-protected accounts with `ProxmoxError::TfaUnsupported`.
//! - Scrubs secrets from all error messages.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::{Duration, Instant};

use mannie_plugin_sdk::contracts::Secret;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, SET_COOKIE};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::model::{ProxmoxError, ServerRecord, is_valid_node};
use crate::tls::{build_tls_config, classify_error_chain};

/// Shared `User-Agent` header sent on Proxmox REST and VNC WebSocket requests.
pub(crate) const USER_AGENT: &str = concat!("Mannie/", env!("CARGO_PKG_VERSION"));

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

/// Maximum number of reverse-proxy cookies retained per Proxmox server.
const MAX_PROXY_COOKIES: usize = 32;

/// Maximum byte length of a stored reverse-proxy cookie name.
const MAX_COOKIE_NAME_BYTES: usize = 128;

/// Maximum byte length of a stored reverse-proxy cookie value.
const MAX_COOKIE_VALUE_BYTES: usize = 4096;

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

/// Short-lived VNC WebSocket proxy ticket returned by `POST .../vncproxy`,
/// paired with the active `Cookie` header (`PVEAuthCookie` plus any stored
/// reverse-proxy session cookies) captured from the same authenticated request.
#[derive(Debug)]
pub(crate) struct VncProxyTicket {
    pub(crate) port: u16,
    pub(crate) ticket: Zeroizing<String>,
    pub(crate) password: Zeroizing<String>,
    pub(crate) cookie_header: Zeroizing<String>,
}

/// Cached Proxmox authentication ticket kept in Rust memory only.
#[derive(Debug)]
struct CachedTicket {
    ticket: Zeroizing<String>,
    csrf: Zeroizing<String>,
    obtained_at: Instant,
}

/// Key for one server-origin reverse-proxy cookie (`Set-Cookie`) retained in memory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CookieKey {
    name: String,
    path: String,
}

/// Per-server Proxmox VE REST API client with shared TLS verification,
/// in-memory ticket cache, and origin-scoped reverse-proxy cookies.
#[derive(Debug)]
pub(crate) struct ProxmoxApiClient {
    host: String,
    endpoint: String,
    username: String,
    http: reqwest::Client,
    ticket: Mutex<Option<CachedTicket>>,
    proxy_cookies: StdMutex<BTreeMap<CookieKey, Zeroizing<String>>>,
}

impl ProxmoxApiClient {
    /// Builds a REST client bound to `server`'s canonical endpoint and TLS trust.
    pub(crate) fn new(server: &ServerRecord) -> Result<Self, ProxmoxError> {
        let tls_config = build_tls_config(
            server.pinned_cert_sha256.as_deref(),
            vec![b"http/1.1".to_vec()],
        )?;
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .tls_backend_preconfigured(tls_config)
            .https_only(true)
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
            host: server.host.to_ascii_lowercase(),
            endpoint: server.endpoint(),
            username: server.username.clone(),
            http,
            ticket: Mutex::new(None),
            proxy_cookies: StdMutex::new(BTreeMap::new()),
        })
    }

    /// Clears the cached ticket and stored reverse-proxy cookies from memory immediately.
    pub(crate) async fn clear_ticket(&self) {
        let mut guard = self.ticket.lock().await;
        *guard = None;
        lock_cookies(&self.proxy_cookies).clear();
    }

    /// Scrubs all known Proxmox and reverse-proxy secrets from a VNC error message.
    pub(crate) async fn scrub_vnc_error(
        &self,
        detail: &str,
        password: &Secret,
        proxy: &VncProxyTicket,
        encoded_ticket: &str,
    ) -> String {
        let cached = {
            let guard = self.ticket.lock().await;
            guard.as_ref().map(|t| (t.ticket.clone(), t.csrf.clone()))
        };
        let proxy_cookies = self.proxy_cookie_values();
        let mut owned_secrets: Vec<&str> = vec![
            password.expose(),
            proxy.ticket.as_str(),
            encoded_ticket,
            proxy.password.as_str(),
            proxy.cookie_header.as_str(),
        ];

        if let Some((ref ticket, ref csrf)) = cached {
            owned_secrets.push(ticket.as_str());
            owned_secrets.push(csrf.as_str());
        }

        for cookie_val in &proxy_cookies {
            owned_secrets.push(cookie_val.as_str());
        }

        scrub_secrets(detail, &owned_secrets)
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
    /// obtain a short-lived WebSocket port, `vncticket`, RFB password, and the
    /// active `Cookie` header (`PVEAuthCookie` plus matching reverse-proxy
    /// session cookies).
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
        let (data, auth_ticket) = self
            .request_json_with_ticket(reqwest::Method::POST, &path, Some(&params), password)
            .await?;
        let parsed: VncProxyResponse =
            serde_json::from_value(data).map_err(|error| ProxmoxError::ApiError {
                detail: format!("unexpected vncproxy response shape: {error}"),
            })?;
        let port = parsed.port.as_u16().ok_or_else(|| ProxmoxError::ApiError {
            detail: "vncproxy response contained an invalid port".to_owned(),
        })?;

        let ws_path = format!("/api2/json/nodes/{node}/qemu/{vmid}/vncwebsocket");
        let cookie_header = self.cookie_header_for(&ws_path, Some(auth_ticket.as_str()));

        Ok(VncProxyTicket {
            port,
            ticket: Zeroizing::new(parsed.ticket),
            password: Zeroizing::new(parsed.password.unwrap_or_default()),
            cookie_header,
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
        let (data, _ticket) = self
            .request_json_with_ticket(method, path_and_query, form_params, password)
            .await?;
        Ok(data)
    }

    /// Sends one authenticated Proxmox JSON API request and returns both the
    /// decoded `data` payload and the `PVEAuthCookie` ticket used by the
    /// winning attempt.
    async fn request_json_with_ticket(
        &self,
        method: reqwest::Method,
        path_and_query: &str,
        form_params: Option<&[(&str, String)]>,
        password: &Secret,
    ) -> Result<(Value, Zeroizing<String>), ProxmoxError> {
        let request_path = path_and_query
            .split_once('?')
            .map_or(path_and_query, |(path, _)| path);
        let mut force_login = false;

        for attempt in 0..2 {
            let (ticket, csrf) = self.ensure_ticket(password, force_login).await?;
            let url = format!("{}{path_and_query}", self.endpoint);
            let mut headers = HeaderMap::new();
            let cookie_header = self.cookie_header_for(request_path, Some(ticket.as_str()));
            let cookie_val = HeaderValue::from_str(cookie_header.as_str()).map_err(|_err| {
                ProxmoxError::ApiError {
                    detail: "invalid Proxmox auth cookie header".to_owned(),
                }
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

            let proxy_values = self.proxy_cookie_values();
            let response = req.send().await.map_err(|error| {
                let secrets = collect_secrets(&[password.expose(), &ticket, &csrf], &proxy_values);
                map_reqwest_error(&error, &secrets)
            })?;
            self.store_set_cookie_headers(request_path, response.headers());
            let status = response.status();

            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                force_login = true;
                continue;
            }

            let updated_proxy_values = self.proxy_cookie_values();
            let secrets =
                collect_secrets(&[password.expose(), &ticket, &csrf], &updated_proxy_values);
            let bytes = read_bounded_body(response, &secrets).await?;
            let data = parse_api_response(status, &bytes, &secrets)?;
            return Ok((data, ticket));
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
        const TICKET_PATH: &str = "/api2/json/access/ticket";

        let url = format!("{}{TICKET_PATH}", self.endpoint);
        let body = encode_form(&[
            ("username", self.username.clone()),
            ("password", credential.to_owned()),
        ]);

        let mut req = self
            .http
            .post(&url)
            .header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/x-www-form-urlencoded"),
            )
            .body(body);
        let existing_cookies = self.cookie_header_for(TICKET_PATH, None);
        if !existing_cookies.is_empty()
            && let Ok(cookie_val) = HeaderValue::from_str(existing_cookies.as_str())
        {
            req = req.header(COOKIE, cookie_val);
        }

        let proxy_values = self.proxy_cookie_values();
        let response = req.send().await.map_err(|error| {
            let secrets =
                collect_secrets(&[password_for_scrub.expose(), credential], &proxy_values);
            map_reqwest_error(&error, &secrets)
        })?;
        self.store_set_cookie_headers(TICKET_PATH, response.headers());
        let status = response.status();
        let updated_proxy_values = self.proxy_cookie_values();
        let secrets = collect_secrets(
            &[password_for_scrub.expose(), credential],
            &updated_proxy_values,
        );
        let bytes = read_bounded_body(response, &secrets).await?;

        if status == StatusCode::UNAUTHORIZED {
            return Err(ProxmoxError::AuthenticationFailed);
        }

        let data = parse_api_response(status, &bytes, &secrets)?;
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

    fn store_set_cookie_headers(&self, request_path: &str, headers: &HeaderMap) {
        let mut cookies = lock_cookies(&self.proxy_cookies);
        for value in headers.get_all(SET_COOKIE) {
            let Ok(raw) = value.to_str() else {
                continue;
            };
            apply_set_cookie(&mut cookies, &self.host, request_path, raw);
        }
    }

    fn cookie_header_for(
        &self,
        request_path: &str,
        auth_ticket: Option<&str>,
    ) -> Zeroizing<String> {
        let cookies = lock_cookies(&self.proxy_cookies);
        format_cookie_header(auth_ticket, &cookies, request_path)
    }

    fn proxy_cookie_values(&self) -> Vec<Zeroizing<String>> {
        lock_cookies(&self.proxy_cookies)
            .values()
            .cloned()
            .collect()
    }
}

fn lock_cookies<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn collect_secrets<'a>(base: &[&'a str], extra: &'a [Zeroizing<String>]) -> Vec<&'a str> {
    let mut out = Vec::with_capacity(base.len() + extra.len());
    out.extend_from_slice(base);
    for value in extra {
        out.push(value.as_str());
    }
    out
}

/// Parses one `Set-Cookie` header for `server_host` / `request_path` and
/// updates `cookies`, ignoring `PVEAuthCookie` (managed from `CachedTicket`)
/// and rejecting foreign domains or unsafe bytes.
fn apply_set_cookie(
    cookies: &mut BTreeMap<CookieKey, Zeroizing<String>>,
    server_host: &str,
    request_path: &str,
    raw: &str,
) {
    let mut parts = raw.split(';');
    let Some(pair) = parts.next() else {
        return;
    };
    let Some((raw_name, raw_value)) = pair.split_once('=') else {
        return;
    };
    let name = raw_name.trim();
    let value = raw_value.trim();

    if !is_valid_cookie_name(name) || name.eq_ignore_ascii_case("pveauthcookie") {
        return;
    }

    let mut path: Option<String> = None;
    let mut delete = false;

    for attr in parts {
        let attr = attr.trim();

        if attr.is_empty() {
            continue;
        }

        let Some((attr_name, attr_val)) = attr.split_once('=') else {
            continue;
        };
        let attr_name = attr_name.trim();
        let attr_val = attr_val.trim();

        if attr_name.eq_ignore_ascii_case("path") {
            if attr_val.starts_with('/') {
                path = Some(attr_val.to_owned());
            }
        } else if attr_name.eq_ignore_ascii_case("domain") {
            if !cookie_domain_matches(server_host, attr_val) {
                return;
            }
        } else if attr_name.eq_ignore_ascii_case("max-age")
            && let Ok(seconds) = attr_val.parse::<i64>()
            && seconds <= 0
        {
            delete = true;
        }
    }

    let resolved_path = path.unwrap_or_else(|| default_cookie_path(request_path));
    let key = CookieKey {
        name: name.to_owned(),
        path: resolved_path,
    };

    if delete || value.is_empty() {
        cookies.remove(&key);
        return;
    }

    if !is_valid_cookie_value(value) {
        return;
    }

    let at_capacity = !cookies.contains_key(&key) && cookies.len() >= MAX_PROXY_COOKIES;

    if at_capacity {
        return;
    }

    cookies.insert(key, Zeroizing::new(value.to_owned()));
}

fn format_cookie_header(
    auth_ticket: Option<&str>,
    cookies: &BTreeMap<CookieKey, Zeroizing<String>>,
    request_path: &str,
) -> Zeroizing<String> {
    let mut matching: Vec<(&CookieKey, &Zeroizing<String>)> = cookies
        .iter()
        .filter(|(key, _)| cookie_path_matches(&key.path, request_path))
        .collect();
    // RFC 6265 §5.4: more specific (longer) cookie paths precede shorter ones.
    matching.sort_by(|(left, _), (right, _)| {
        right
            .path
            .len()
            .cmp(&left.path.len())
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut header = Zeroizing::new(String::new());

    if let Some(ticket) = auth_ticket {
        header.push_str("PVEAuthCookie=");
        header.push_str(ticket);
    }

    for (key, value) in matching {
        if !header.is_empty() {
            header.push_str("; ");
        }

        header.push_str(&key.name);
        header.push('=');
        header.push_str(value.as_str());
    }

    header
}

fn is_valid_cookie_name(name: &str) -> bool {
    let len_ok = !name.is_empty() && name.len() <= MAX_COOKIE_NAME_BYTES;
    let chars_ok = name.bytes().all(|b| {
        b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'!' | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'|'
                    | b'~'
            )
    });
    len_ok && chars_ok
}

fn is_valid_cookie_value(value: &str) -> bool {
    let len_ok = !value.is_empty() && value.len() <= MAX_COOKIE_VALUE_BYTES;
    let chars_ok = value
        .bytes()
        .all(|b| (0x20..=0x7e).contains(&b) && b != b';');
    len_ok && chars_ok
}

fn cookie_domain_matches(server_host: &str, raw_domain: &str) -> bool {
    let domain = raw_domain.trim_start_matches('.').to_ascii_lowercase();

    if domain.is_empty() {
        return false;
    }

    let host = server_host.to_ascii_lowercase();

    if host == domain {
        return true;
    }

    if host.parse::<IpAddr>().is_ok() {
        return false;
    }

    host.len() > domain.len()
        && host.ends_with(&domain)
        && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
}

/// Computes RFC 6265 §5.1.4 default-path from `request_path`.
fn default_cookie_path(request_path: &str) -> String {
    if !request_path.starts_with('/') {
        return "/".to_owned();
    }

    match request_path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(pos) => request_path[..pos].to_owned(),
    }
}

/// Matches a stored cookie path against `request_path` per RFC 6265 §5.1.4.
fn cookie_path_matches(cookie_path: &str, request_path: &str) -> bool {
    if request_path == cookie_path {
        return true;
    }

    if !request_path.starts_with(cookie_path) {
        return false;
    }

    cookie_path.ends_with('/') || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')
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
    secrets: &[&str],
) -> Result<Vec<u8>, ProxmoxError> {
    let mut out = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| map_reqwest_error(&error, secrets))?
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
    secrets: &[&str],
) -> Result<Value, ProxmoxError> {
    if status == StatusCode::UNAUTHORIZED {
        return Err(ProxmoxError::AuthenticationFailed);
    }

    if status == StatusCode::FORBIDDEN {
        let detail = extract_error_detail(status, bytes, secrets);
        return Err(ProxmoxError::PermissionDenied { detail });
    }

    if status == StatusCode::NOT_FOUND {
        return Err(ProxmoxError::VmNotFound);
    }

    if !status.is_success() {
        let detail = extract_error_detail(status, bytes, secrets);
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

fn map_reqwest_error(error: &reqwest::Error, secrets: &[&str]) -> ProxmoxError {
    if error.is_timeout() {
        return ProxmoxError::Timeout;
    }

    if let Some(tls_err) = classify_error_chain(error) {
        return tls_err;
    }

    let detail = scrub_secrets(&error.to_string(), secrets);
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
/// Advertises `http/1.1` via ALPN to match `ProxmoxApiClient` and RFC 6455 HTTP/1.1 upgrades.
pub(crate) fn build_vnc_tls_config(
    server: &ServerRecord,
) -> Result<Arc<rustls::ClientConfig>, ProxmoxError> {
    build_tls_config(
        server.pinned_cert_sha256.as_deref(),
        vec![b"http/1.1".to_vec()],
    )
    .map(Arc::new)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        apply_set_cookie, build_vnc_tls_config, cookie_domain_matches, cookie_path_matches,
        default_cookie_path, format_cookie_header,
    };
    use crate::model::{ServerRecord, canonical_endpoint, canonical_host_header};

    #[test]
    fn canonical_host_header_omits_default_https_port_and_brackets_ipv6() {
        assert_eq!(
            canonical_host_header("pve.work.local", 443),
            "pve.work.local"
        );
        assert_eq!(
            canonical_host_header("pve.work.local", 8006),
            "pve.work.local:8006"
        );
        assert_eq!(canonical_host_header("::1", 443), "[::1]");
        assert_eq!(canonical_host_header("::1", 8006), "[::1]:8006");
        assert_eq!(
            canonical_endpoint("pve.work.local", 443),
            "https://pve.work.local"
        );
        assert_eq!(
            canonical_endpoint("pve.work.local", 8006),
            "https://pve.work.local:8006"
        );
    }

    #[test]
    fn vnc_tls_config_advertises_http_1_1_alpn() {
        let server = ServerRecord {
            id: "00000000-0000-7000-8000-000000000001".to_owned(),
            name: "Work".to_owned(),
            host: "pve.work.local".to_owned(),
            port: 443,
            username: "lab@pve".to_owned(),
            password_set: true,
            pinned_cert_sha256: Some("aa".repeat(32)),
            pinned_cert_subject: None,
            pinned_cert_at: None,
            note: String::new(),
        };
        let cfg = build_vnc_tls_config(&server).expect("build_vnc_tls_config");
        assert_eq!(cfg.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }

    #[test]
    fn proxy_cookies_parse_match_and_expire_without_overwriting_pve_auth() {
        let mut cookies = BTreeMap::new();

        apply_set_cookie(
            &mut cookies,
            "pve.work.local",
            "/api2/json/access/ticket",
            "SERVERID=pve1; Path=/; HttpOnly; Secure",
        );
        apply_set_cookie(
            &mut cookies,
            "pve.work.local",
            "/api2/json/access/ticket",
            "PVEAuthCookie=spoofed; Path=/",
        );
        apply_set_cookie(
            &mut cookies,
            "pve.work.local",
            "/api2/json/nodes/pve1/qemu/768/vncproxy",
            "WSSTICKY=node1; Path=/api2/json/nodes/pve1; Secure",
        );
        apply_set_cookie(
            &mut cookies,
            "pve.work.local",
            "/api2/json/access/ticket",
            "FOREIGN=bad; Domain=evil.example; Path=/",
        );
        apply_set_cookie(
            &mut cookies,
            "pve.work.local",
            "/api2/json/access/ticket",
            "OTHERPATH=ignored; Path=/other",
        );

        let ws_header = format_cookie_header(
            Some("PVE:lab@pve:1234::sig"),
            &cookies,
            "/api2/json/nodes/pve1/qemu/768/vncwebsocket",
        );
        assert_eq!(
            ws_header.as_str(),
            "PVEAuthCookie=PVE:lab@pve:1234::sig; WSSTICKY=node1; SERVERID=pve1"
        );

        let other_node_header = format_cookie_header(
            Some("PVE:lab@pve:1234::sig"),
            &cookies,
            "/api2/json/nodes/pve2/qemu/769/vncwebsocket",
        );
        assert_eq!(
            other_node_header.as_str(),
            "PVEAuthCookie=PVE:lab@pve:1234::sig; SERVERID=pve1"
        );

        // Max-Age=0 deletes the stored proxy cookie.
        apply_set_cookie(
            &mut cookies,
            "pve.work.local",
            "/api2/json/access/ticket",
            "SERVERID=deleted; Path=/; Max-Age=0",
        );
        let after_delete = format_cookie_header(
            Some("PVE:lab@pve:1234::sig"),
            &cookies,
            "/api2/json/nodes/pve2/qemu/769/vncwebsocket",
        );
        assert_eq!(after_delete.as_str(), "PVEAuthCookie=PVE:lab@pve:1234::sig");
    }

    #[test]
    fn cookie_domain_and_path_helpers_follow_rfc6265() {
        assert!(cookie_domain_matches("pve.work.local", "pve.work.local"));
        assert!(cookie_domain_matches("pve.work.local", ".work.local"));
        assert!(!cookie_domain_matches("pve.work.local", "other.local"));
        assert!(!cookie_domain_matches("192.168.1.50", "168.1.50"));
        assert!(cookie_domain_matches("192.168.1.50", "192.168.1.50"));

        assert_eq!(
            default_cookie_path("/api2/json/nodes/pve1/qemu/768/vncproxy"),
            "/api2/json/nodes/pve1/qemu/768"
        );
        assert_eq!(default_cookie_path("/ticket"), "/");

        assert!(cookie_path_matches("/", "/api2/json/nodes/pve1"));
        assert!(cookie_path_matches("/api2/json", "/api2/json/nodes/pve1"));
        assert!(!cookie_path_matches("/api2/json", "/api2/json-other"));
    }

    #[tokio::test]
    async fn scrub_vnc_error_redacts_encoded_vncticket_and_proxy_cookies() {
        use mannie_plugin_sdk::contracts::Secret;
        use zeroize::Zeroizing;

        use super::{ProxmoxApiClient, VncProxyTicket, encode_vnc_ticket};

        let server = ServerRecord {
            id: "00000000-0000-7000-8000-000000000002".to_owned(),
            name: "Work".to_owned(),
            host: "pve.work.local".to_owned(),
            port: 443,
            username: "lab@pve".to_owned(),
            password_set: true,
            pinned_cert_sha256: Some("bb".repeat(32)),
            pinned_cert_subject: None,
            pinned_cert_at: None,
            note: String::new(),
        };
        let api = ProxmoxApiClient::new(&server).expect("build api client");
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::SET_COOKIE,
            reqwest::header::HeaderValue::from_static("SERVERID=STICKY_NODE_9999; Path=/"),
        );
        api.store_set_cookie_headers("/api2/json/access/ticket", &headers);

        let raw_vnc_ticket = "PVEVNC:6745ABCD::abc+def/ghi==";
        let encoded_vnc_ticket = encode_vnc_ticket(raw_vnc_ticket);
        let password = Secret::from("SUPER_SECRET_PASSWORD".to_owned());
        let proxy = VncProxyTicket {
            port: 5900,
            ticket: Zeroizing::new(raw_vnc_ticket.to_owned()),
            password: Zeroizing::new("RFB_PASS_1234".to_owned()),
            cookie_header: Zeroizing::new(
                "PVEAuthCookie=PVE:lab@pve:1234::tok; SERVERID=STICKY_NODE_9999".to_owned(),
            ),
        };
        let raw_error = format!(
            "502 echoing /vncwebsocket?port=5900&vncticket={encoded_vnc_ticket} \
             {raw_vnc_ticket} STICKY_NODE_9999 SUPER_SECRET_PASSWORD RFB_PASS_1234"
        );
        let scrubbed = api
            .scrub_vnc_error(&raw_error, &password, &proxy, &encoded_vnc_ticket)
            .await;

        for secret in [
            raw_vnc_ticket,
            encoded_vnc_ticket.as_str(),
            "STICKY_NODE_9999",
            "SUPER_SECRET_PASSWORD",
            "RFB_PASS_1234",
        ] {
            assert!(
                !scrubbed.contains(secret),
                "secret {secret} leaked in scrubbed detail: {scrubbed}"
            );
        }
    }
}
