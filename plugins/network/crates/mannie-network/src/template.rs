//! Variable and path-parameter interpolation for network requests, tracking
//! secret provenance so every request produces both the real wire values and
//! the strictly `[redacted]` representation for approvals and agent tools
//! (ADR-0075, decisions 2 and 5).

use std::collections::BTreeMap;

use sha2::{Digest as _, Sha256};
use url::Url;

use crate::model::{
    ApiKeyLocation, AuthConfig, BodyConfig, CollectionView, EnvironmentView, KeyValueEntry,
    MAX_BODY_BYTES, NetworkError, RequestView, VariableEntry,
};
use crate::redact::{REDACTED, is_secret_header_name, is_secret_name, percent_encode_rfc3986};

/// Maximum transitive interpolation passes for nested `{{variable}}` templates
/// (such as multi-server `OpenAPI` `base_url` referencing server variables).
const MAX_INTERPOLATION_PASSES: usize = 4;

/// Upper bound on an interpolated string so nested templates cannot expand
/// without limit.
const MAX_INTERPOLATED_BYTES: usize = MAX_BODY_BYTES + 64 * 1024;

/// One resolved variable with its value and whether it is secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedVariable {
    /// Actual value (or empty when unresolved in dry-run redaction mode).
    pub(crate) value: String,
    /// Whether this variable is secret and must appear as `[redacted]` to
    /// agents and in approvals.
    pub(crate) secret: bool,
}

/// Precedence map of active variables for a request.
pub(crate) type VariableScope = BTreeMap<String, ResolvedVariable>;

/// Builds the active variable scope with precedence:
/// runtime overrides (non-secret) > active environment > collection > global.
pub(crate) fn build_variable_scope(
    global_variables: &[VariableEntry],
    collection_variables: &[VariableEntry],
    environment: Option<&EnvironmentView>,
    runtime_overrides: &BTreeMap<String, String>,
) -> VariableScope {
    let mut scope = VariableScope::new();
    apply_variables(&mut scope, global_variables);
    apply_variables(&mut scope, collection_variables);

    if let Some(env) = environment {
        apply_variables(&mut scope, &env.variables);
    }

    for (key, value) in runtime_overrides {
        if !key.trim().is_empty() {
            let secret = is_secret_name(key);
            scope.insert(
                key.trim().to_owned(),
                ResolvedVariable {
                    value: value.clone(),
                    secret,
                },
            );
        }
    }
    scope
}

fn apply_variables(scope: &mut VariableScope, entries: &[VariableEntry]) {
    for entry in entries {
        if !entry.enabled || entry.key.trim().is_empty() {
            continue;
        }

        let secret = entry.secret || is_secret_name(&entry.key);
        scope.insert(
            entry.key.trim().to_owned(),
            ResolvedVariable {
                value: entry.value.clone(),
                secret,
            },
        );
    }
}

/// Result of substituting `{{variable}}` templates in text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interpolated {
    /// Text with real variable values substituted.
    pub(crate) real: String,
    /// Text with secret variables replaced by `[redacted]`.
    pub(crate) redacted: String,
    /// Whether any secret variable participated in the substitution.
    pub(crate) used_secret: bool,
}

/// Replaces `{{name}}` occurrences using `scope`, following nested references
/// up to [`MAX_INTERPOLATION_PASSES`] passes while tracking secret provenance.
/// Unknown placeholders stay as written so callers see unresolved templates
/// clearly.
pub(crate) fn interpolate(template: &str, scope: &VariableScope) -> Interpolated {
    let mut current_real = template.to_owned();
    let mut current_redacted = template.to_owned();
    let mut used_secret = false;

    for _pass in 0..MAX_INTERPOLATION_PASSES {
        let (next_real, real_changed, _) = interpolate_once(&current_real, scope, false);
        let (next_redacted, redacted_changed, pass_secret) =
            interpolate_once(&current_redacted, scope, true);

        if pass_secret {
            used_secret = true;
        }

        if next_real.len() > MAX_INTERPOLATED_BYTES || next_redacted.len() > MAX_INTERPOLATED_BYTES
        {
            break;
        }

        current_real = next_real;
        current_redacted = next_redacted;

        if !real_changed && !redacted_changed {
            break;
        }
    }

    Interpolated {
        real: current_real,
        redacted: current_redacted,
        used_secret,
    }
}

fn interpolate_once(template: &str, scope: &VariableScope, redact: bool) -> (String, bool, bool) {
    let mut output = String::with_capacity(template.len());
    let mut changed = false;
    let mut used_secret = false;
    let mut cursor = 0;

    while let Some(open_rel) = template[cursor..].find("{{") {
        let open = cursor + open_rel;
        output.push_str(&template[cursor..open]);

        let after_open = open + 2;

        if let Some(close_rel) = template[after_open..].find("}}") {
            let close = after_open + close_rel;
            let name = template[after_open..close].trim();

            if let Some(var) = scope.get(name) {
                changed = true;

                if var.secret {
                    used_secret = true;
                }

                if redact && var.secret {
                    output.push_str(REDACTED);
                } else {
                    output.push_str(&var.value);
                }
            } else {
                output.push_str(&template[open..close + 2]);
            }

            cursor = close + 2;
        } else {
            output.push_str(&template[open..]);
            return (output, changed, used_secret);
        }
    }

    output.push_str(&template[cursor..]);
    (output, changed, used_secret)
}

/// Runtime input overrides passed by an agent or workbench caller.
#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeOverrides {
    /// Path parameter overrides (`{id}`).
    pub(crate) path_params: BTreeMap<String, String>,
    /// Query parameter overrides.
    pub(crate) query_params: BTreeMap<String, String>,
    /// Non-secret header overrides.
    pub(crate) headers: BTreeMap<String, String>,
    /// Non-secret variable overrides.
    pub(crate) variables: BTreeMap<String, String>,
    /// Optional raw body override.
    pub(crate) body: Option<String>,
    /// Optional timeout override in seconds.
    pub(crate) timeout_seconds: Option<u32>,
}

/// A fully resolved HTTP request ready to send (with both wire and sanitized
/// views).
#[derive(Debug, Clone)]
pub(crate) struct ResolvedHttpRequest {
    /// Canonical `http` or `https` origin (never contains secrets).
    pub(crate) origin: String,
    /// Real resolved URL sent on the wire.
    pub(crate) real_url: Url,
    /// Sanitized URL shown in approvals, logs and agent outputs.
    pub(crate) redacted_url: String,
    /// Wire headers `(name, value, is_secret)`.
    pub(crate) headers: Vec<(String, String, bool)>,
    /// Wire request body and optional `Content-Type` header.
    pub(crate) body: Option<(Vec<u8>, Option<String>)>,
    /// Sanitized summary JSON for `NetworkRequestAccess::summary`.
    pub(crate) summary_json: String,
    /// Whether any secret variable, secret header, secret query param, auth
    /// credential or secret form entry is attached.
    pub(crate) uses_credentials: bool,
    /// Basic auth `(username, password)` pair if used, so `Redactor` also
    /// scrubs the base64 authorization value.
    pub(crate) basic_auth: Option<(String, String)>,
    /// Effective timeout in seconds.
    pub(crate) timeout_seconds: u32,
}

/// Whether runtime overrides may change the request origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginPolicy {
    /// Owner workbench sends (no agent overrides).
    OwnerWorkbench,
    /// Agent tool calls: runtime overrides may never alter the origin resolved
    /// from the owner's saved collection, request and selected environment
    /// (ADR-0075).
    AgentBoundToOwnerOrigin,
}

/// Why a request template could not be resolved into a valid HTTP URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolveError {
    /// The URL is not a valid `http` or `https` URL, lacks a host or embeds
    /// user information.
    InvalidUrl,
    /// A secret variable was used inside the scheme, host or port.
    SecretInOrigin,
    /// A path parameter contained control characters.
    InvalidPathParam,
    /// An agent override tried to redirect the request to an origin the owner
    /// did not configure on the saved collection, request or environment.
    OriginOverrideRefused,
}

/// Resolves a saved request against its collection, active environment, global
/// variables and optional runtime overrides.
#[expect(
    clippy::too_many_lines,
    reason = "URL, path, query, header, auth and body resolution share one provenance pass"
)]
pub(crate) fn resolve_http_request(
    collection: &CollectionView,
    request: &RequestView,
    environment: Option<&EnvironmentView>,
    global_variables: &[VariableEntry],
    overrides: &RuntimeOverrides,
    origin_policy: OriginPolicy,
) -> Result<ResolvedHttpRequest, ResolveError> {
    let required_owner_origin = match origin_policy {
        OriginPolicy::OwnerWorkbench => None,
        OriginPolicy::AgentBoundToOwnerOrigin => {
            let owner_origin = resolve_owner_configured_origin(
                collection,
                request,
                environment,
                global_variables,
            )?;
            Some(owner_origin)
        }
    };

    let scope = build_variable_scope(
        global_variables,
        &collection.variables,
        environment,
        &overrides.variables,
    );
    let mut uses_credentials = false;
    let mut basic_auth = None;

    // Resolve URL base and template.
    let base_interp = interpolate(collection.base_url.trim(), &scope);
    let url_interp = interpolate(request.url_template.trim(), &scope);
    let mut combined_real = combine_base_and_template(&base_interp.real, &url_interp.real);
    let mut combined_redacted =
        combine_base_and_template(&base_interp.redacted, &url_interp.redacted);

    if base_interp.used_secret || url_interp.used_secret {
        uses_credentials = true;
    }

    // Resolve `{param}` path parameters with RFC 3986 path-segment encoding.
    let mut path_map: BTreeMap<String, (String, bool)> = BTreeMap::new();

    for entry in &request.path_params {
        if !entry.enabled || entry.key.trim().is_empty() {
            continue;
        }

        let interp = interpolate(&entry.value, &scope);
        let secret = entry.secret || is_secret_name(&entry.key) || interp.used_secret;
        path_map.insert(entry.key.trim().to_owned(), (interp.real, secret));
    }

    for (key, value) in &overrides.path_params {
        let interp = interpolate(value, &scope);
        let secret = is_secret_name(key) || interp.used_secret;
        path_map.insert(key.trim().to_owned(), (interp.real, secret));
    }

    let mut summary_path = BTreeMap::new();

    for (key, (value, secret)) in &path_map {
        if value.chars().any(char::is_control) {
            return Err(ResolveError::InvalidPathParam);
        }

        let token = format!("{{{key}}}");

        if combined_real.contains(&token) {
            let encoded = percent_encode_rfc3986(value);
            combined_real = combined_real.replace(&token, &encoded);

            if *secret {
                combined_redacted = combined_redacted.replace(&token, REDACTED);
                summary_path.insert(key.clone(), REDACTED.to_owned());
                uses_credentials = true;
            } else {
                combined_redacted = combined_redacted.replace(&token, &encoded);
                summary_path.insert(key.clone(), value.clone());
            }
        }
    }

    let mut real_url = parse_http_url(&combined_real)?;
    let mut redacted_url = match parse_http_url(&combined_redacted) {
        Ok(url) => url,
        Err(_err) if combined_redacted.contains(REDACTED) => {
            return Err(ResolveError::SecretInOrigin);
        }
        Err(err) => return Err(err),
    };

    if let Some(expected_origin) = required_owner_origin
        && real_url.origin().ascii_serialization() != expected_origin
    {
        return Err(ResolveError::OriginOverrideRefused);
    }

    // Reject secret values in scheme/host/port so canonical origin is never secret.
    let origin = real_url.origin().ascii_serialization();
    let redacted_origin = redacted_url.origin().ascii_serialization();

    if origin != redacted_origin {
        return Err(ResolveError::SecretInOrigin);
    }

    // Collect query parameters: collection defaults -> request params -> overrides.
    let mut query_entries: Vec<(String, String, String, bool)> = Vec::new();
    collect_query_entries(&collection.query_params, &scope, &mut query_entries);
    collect_query_entries(&request.query_params, &scope, &mut query_entries);

    for (key, value) in &overrides.query_params {
        if key.trim().is_empty() {
            continue;
        }

        let interp = interpolate(value, &scope);
        let secret = is_secret_name(key) || interp.used_secret;
        let redacted_val = if secret {
            REDACTED.to_owned()
        } else {
            interp.redacted
        };
        query_entries.retain(|(existing, ..)| existing != key.trim());
        query_entries.push((key.trim().to_owned(), interp.real, redacted_val, secret));
    }

    // Effective auth: inherit falls back to collection auth.
    let effective_auth = match &request.auth {
        AuthConfig::Inherit => &collection.auth,
        other => other,
    };

    // Collect headers: collection defaults -> request headers -> overrides.
    let mut header_map: BTreeMap<String, (String, String, bool)> = BTreeMap::new();
    collect_header_entries(&collection.headers, &scope, &mut header_map);
    collect_header_entries(&request.headers, &scope, &mut header_map);

    for (key, value) in &overrides.headers {
        let trimmed = key.trim();

        if trimmed.is_empty() {
            continue;
        }

        let interp = interpolate(value, &scope);
        let secret = is_secret_header_name(trimmed) || interp.used_secret;
        header_map.insert(
            trimmed.to_ascii_lowercase(),
            (trimmed.to_owned(), interp.real, secret),
        );
    }

    // Apply authentication to headers or query parameters.
    match effective_auth {
        AuthConfig::Inherit | AuthConfig::None => {}
        AuthConfig::Bearer { token } => {
            let interp = interpolate(token, &scope);

            if !interp.real.is_empty() {
                uses_credentials = true;
                header_map.insert(
                    "authorization".to_owned(),
                    (
                        "Authorization".to_owned(),
                        format!("Bearer {}", interp.real),
                        true,
                    ),
                );
            }
        }
        AuthConfig::Basic { username, password } => {
            let user_interp = interpolate(username, &scope);
            let pass_interp = interpolate(password, &scope);

            if !user_interp.real.is_empty() || !pass_interp.real.is_empty() {
                use base64::Engine as _;
                let encoded = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", user_interp.real, pass_interp.real));
                uses_credentials = true;
                basic_auth = Some((user_interp.real, pass_interp.real));
                header_map.insert(
                    "authorization".to_owned(),
                    ("Authorization".to_owned(), format!("Basic {encoded}"), true),
                );
            }
        }
        AuthConfig::ApiKey {
            name,
            value,
            location,
        } => {
            let interp = interpolate(value, &scope);

            if !name.trim().is_empty() && !interp.real.is_empty() {
                uses_credentials = true;

                match location {
                    ApiKeyLocation::Header => {
                        header_map.insert(
                            name.trim().to_ascii_lowercase(),
                            (name.trim().to_owned(), interp.real, true),
                        );
                    }
                    ApiKeyLocation::Query => {
                        query_entries.push((
                            name.trim().to_owned(),
                            interp.real,
                            REDACTED.to_owned(),
                            true,
                        ));
                    }
                }
            }
        }
    }

    // Append query parameters to both real and redacted URLs, and redact any
    // pre-existing secret query parameters already present in `url_template`.
    let existing_pairs: Vec<(String, String)> = redacted_url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if !existing_pairs.is_empty() {
        redacted_url.set_query(None);
        let mut pairs = redacted_url.query_pairs_mut();

        for (k, v) in existing_pairs {
            if is_secret_name(&k) {
                uses_credentials = true;
                pairs.append_pair(&k, REDACTED);
            } else {
                pairs.append_pair(&k, &v);
            }
        }
    }

    let mut summary_query = BTreeMap::new();

    if !query_entries.is_empty() {
        let mut real_pairs = real_url.query_pairs_mut();
        let mut red_pairs = redacted_url.query_pairs_mut();

        for (key, real_val, red_val, secret) in &query_entries {
            if *secret {
                uses_credentials = true;
            }

            real_pairs.append_pair(key, real_val);
            red_pairs.append_pair(key, red_val);
            summary_query.insert(key.clone(), red_val.clone());
        }
    }

    let mut wire_headers = Vec::with_capacity(header_map.len());
    let mut redacted_headers = BTreeMap::new();

    for (name, value, secret) in header_map.into_values() {
        if secret {
            uses_credentials = true;
            redacted_headers.insert(name.clone(), REDACTED.to_owned());
        } else {
            redacted_headers.insert(name.clone(), value.clone());
        }

        wire_headers.push((name, value, secret));
    }

    // Resolve body.
    let (body, body_summary) = resolve_body(
        &request.body,
        overrides.body.as_deref(),
        &scope,
        &mut uses_credentials,
    );

    let mut summary = serde_json::Map::new();

    if let Some(env) = environment {
        summary.insert("environment".to_owned(), serde_json::json!(env.name));
    }
    if !summary_path.is_empty() {
        summary.insert("pathParams".to_owned(), serde_json::json!(summary_path));
    }
    if !summary_query.is_empty() {
        summary.insert("query".to_owned(), serde_json::json!(summary_query));
    }
    if !redacted_headers.is_empty() {
        summary.insert("headers".to_owned(), serde_json::json!(redacted_headers));
    }
    if let Some(body_meta) = body_summary {
        summary.insert("body".to_owned(), body_meta);
    }

    let summary_json = serde_json::Value::Object(summary).to_string();
    let timeout_seconds = overrides
        .timeout_seconds
        .unwrap_or(request.timeout_seconds)
        .clamp(
            crate::model::MIN_TIMEOUT_SECONDS,
            crate::model::MAX_TIMEOUT_SECONDS,
        );

    Ok(ResolvedHttpRequest {
        origin,
        real_url,
        redacted_url: redacted_url.to_string(),
        headers: wire_headers,
        body,
        summary_json,
        uses_credentials,
        basic_auth,
        timeout_seconds,
    })
}

fn combine_base_and_template(base: &str, template: &str) -> String {
    let trimmed = template.trim();

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_owned();
    }

    let base_trimmed = base.trim();

    if base_trimmed.is_empty() {
        return trimmed.to_owned();
    }
    if trimmed.is_empty() {
        return base_trimmed.to_owned();
    }

    let base_clean = base_trimmed.trim_end_matches('/');

    if trimmed.starts_with('/') || trimmed.starts_with('?') {
        format!("{base_clean}{trimmed}")
    } else {
        format!("{base_clean}/{trimmed}")
    }
}

fn parse_http_url(raw: &str) -> Result<Url, ResolveError> {
    if raw.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        return Err(ResolveError::InvalidUrl);
    }

    let mut parsed = Url::parse(raw).map_err(|_parse| ResolveError::InvalidUrl)?;
    let http_scheme = matches!(parsed.scheme(), "http" | "https");
    let has_host = parsed.host().is_some();
    let has_userinfo = !parsed.username().is_empty() || parsed.password().is_some();

    if !http_scheme || !has_host || has_userinfo {
        return Err(ResolveError::InvalidUrl);
    }

    // Fragments are never sent over HTTP; strip so canonicalization matches.
    parsed.set_fragment(None);
    Ok(parsed)
}

fn collect_query_entries(
    entries: &[KeyValueEntry],
    scope: &VariableScope,
    out: &mut Vec<(String, String, String, bool)>,
) {
    for entry in entries {
        if !entry.enabled || entry.key.trim().is_empty() {
            continue;
        }

        let key = entry.key.trim().to_owned();
        let interp = interpolate(&entry.value, scope);
        let secret = entry.secret || is_secret_name(&key) || interp.used_secret;
        let red_val = if secret {
            REDACTED.to_owned()
        } else {
            interp.redacted
        };
        out.push((key, interp.real, red_val, secret));
    }
}

fn collect_header_entries(
    entries: &[KeyValueEntry],
    scope: &VariableScope,
    out: &mut BTreeMap<String, (String, String, bool)>,
) {
    for entry in entries {
        if !entry.enabled || entry.key.trim().is_empty() {
            continue;
        }

        let name = entry.key.trim().to_owned();
        let interp = interpolate(&entry.value, scope);
        let secret = entry.secret || is_secret_header_name(&name) || interp.used_secret;
        out.insert(name.to_ascii_lowercase(), (name, interp.real, secret));
    }
}

type WireBody = (Vec<u8>, Option<String>);
type ResolvedBody = (Option<WireBody>, Option<serde_json::Value>);

fn resolve_body(
    body: &BodyConfig,
    override_body: Option<&str>,
    scope: &VariableScope,
    uses_credentials: &mut bool,
) -> ResolvedBody {
    if let Some(raw_override) = override_body {
        return resolve_override_body(body, raw_override, scope, uses_credentials);
    }

    match body {
        BodyConfig::None => (None, None),
        BodyConfig::Json { content } => {
            let interp = interpolate(content, scope);

            if interp.used_secret {
                *uses_credentials = true;
            }

            let bytes = interp.real.into_bytes();
            let summary = serde_json::json!({
                "kind": "json",
                "bytes": bytes.len(),
                "sha256": sha256_hex(interp.redacted.as_bytes()),
                "preview": preview_text(&interp.redacted),
            });
            (
                Some((bytes, Some("application/json".to_owned()))),
                Some(summary),
            )
        }
        BodyConfig::Text {
            content,
            content_type,
        } => {
            let interp = interpolate(content, scope);

            if interp.used_secret {
                *uses_credentials = true;
            }

            let ct = if content_type.trim().is_empty() {
                "text/plain; charset=utf-8".to_owned()
            } else {
                content_type.trim().to_owned()
            };
            let bytes = interp.real.into_bytes();
            let summary = serde_json::json!({
                "kind": "text",
                "contentType": ct,
                "bytes": bytes.len(),
                "sha256": sha256_hex(interp.redacted.as_bytes()),
                "preview": preview_text(&interp.redacted),
            });
            (Some((bytes, Some(ct))), Some(summary))
        }
        BodyConfig::FormUrlEncoded { entries } => {
            resolve_form_body(entries, scope, uses_credentials)
        }
    }
}

fn resolve_override_body(
    body: &BodyConfig,
    raw_override: &str,
    scope: &VariableScope,
    uses_credentials: &mut bool,
) -> ResolvedBody {
    let interp = interpolate(raw_override, scope);

    if interp.used_secret {
        *uses_credentials = true;
    }

    let content_type = match body {
        BodyConfig::Text { content_type, .. } if !content_type.trim().is_empty() => {
            Some(content_type.trim().to_owned())
        }
        BodyConfig::FormUrlEncoded { .. } => Some("application/x-www-form-urlencoded".to_owned()),
        _other => Some("application/json".to_owned()),
    };
    let bytes = interp.real.into_bytes();
    let summary = serde_json::json!({
        "kind": "override",
        "bytes": bytes.len(),
        "sha256": sha256_hex(interp.redacted.as_bytes()),
        "preview": preview_text(&interp.redacted),
    });
    (Some((bytes, content_type)), Some(summary))
}

fn resolve_form_body(
    entries: &[KeyValueEntry],
    scope: &VariableScope,
    uses_credentials: &mut bool,
) -> ResolvedBody {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    let mut summary_fields = BTreeMap::new();

    for entry in entries {
        if !entry.enabled || entry.key.trim().is_empty() {
            continue;
        }

        let key = entry.key.trim();
        let interp = interpolate(&entry.value, scope);
        let secret = entry.secret || is_secret_name(key) || interp.used_secret;

        if secret {
            *uses_credentials = true;
            summary_fields.insert(key.to_owned(), REDACTED.to_owned());
        } else {
            summary_fields.insert(key.to_owned(), interp.redacted);
        }

        serializer.append_pair(key, &interp.real);
    }

    let encoded = serializer.finish().into_bytes();
    let digest_source = serde_json::to_string(&summary_fields).unwrap_or_default();
    let summary = serde_json::json!({
        "kind": "form_urlencoded",
        "bytes": encoded.len(),
        "sha256": sha256_hex(digest_source.as_bytes()),
        "fields": summary_fields,
    });
    (
        Some((
            encoded,
            Some("application/x-www-form-urlencoded".to_owned()),
        )),
        Some(summary),
    )
}

fn resolve_owner_configured_origin(
    collection: &CollectionView,
    request: &RequestView,
    environment: Option<&EnvironmentView>,
    global_variables: &[VariableEntry],
) -> Result<String, ResolveError> {
    let empty_overrides = BTreeMap::new();
    let owner_scope = build_variable_scope(
        global_variables,
        &collection.variables,
        environment,
        &empty_overrides,
    );
    let base_interp = interpolate(collection.base_url.trim(), &owner_scope);
    let url_interp = interpolate(request.url_template.trim(), &owner_scope);
    let mut combined_real = combine_base_and_template(&base_interp.real, &url_interp.real);
    let mut combined_redacted =
        combine_base_and_template(&base_interp.redacted, &url_interp.redacted);

    for entry in &request.path_params {
        if !entry.enabled || entry.key.trim().is_empty() {
            continue;
        }

        let interp = interpolate(&entry.value, &owner_scope);
        let secret = entry.secret || is_secret_name(&entry.key) || interp.used_secret;
        let token = format!("{{{}}}", entry.key.trim());

        if combined_real.contains(&token) {
            let encoded = percent_encode_rfc3986(&interp.real);
            combined_real = combined_real.replace(&token, &encoded);
            combined_redacted =
                combined_redacted.replace(&token, if secret { REDACTED } else { &encoded });
        }
    }

    let real_url = parse_http_url(&combined_real).map_err(|err| match err {
        ResolveError::InvalidUrl => ResolveError::OriginOverrideRefused,
        other => other,
    })?;
    let redacted_url = match parse_http_url(&combined_redacted) {
        Ok(url) => url,
        Err(_err) if combined_redacted.contains(REDACTED) => {
            return Err(ResolveError::SecretInOrigin);
        }
        Err(_err) => return Err(ResolveError::OriginOverrideRefused),
    };
    let origin = real_url.origin().ascii_serialization();

    if origin != redacted_url.origin().ascii_serialization() {
        return Err(ResolveError::SecretInOrigin);
    }

    Ok(origin)
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);

    for byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }

    out
}

/// Validates a saved `base_url` or `url_template` before storing it in SQLite:
/// refuses embedded userinfo (`user:pass@`) and extracts any secret-named query
/// parameters so literal credentials move into the OS credential store instead
/// of SQLite or chat transcripts.
pub(crate) fn sanitize_saved_url(raw: &str) -> Result<(String, Vec<KeyValueEntry>), NetworkError> {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return Ok((String::new(), Vec::new()));
    }
    if url_embeds_userinfo(trimmed) {
        return Err(NetworkError::InvalidUrl);
    }

    let Some((before_query, raw_query)) = trimmed.split_once('?') else {
        return Ok((trimmed.to_owned(), Vec::new()));
    };
    let (query_part, fragment_suffix) = match raw_query.split_once('#') {
        Some((q, f)) => (q, Some(f)),
        None => (raw_query, None),
    };
    let mut kept_pairs = Vec::new();
    let mut extracted = Vec::new();

    for part in query_part.split('&') {
        if part.is_empty() {
            continue;
        }

        let (raw_key, raw_val) = part.split_once('=').unwrap_or((part, ""));
        let decoded_key = decode_query_component(raw_key);

        if is_secret_name(&decoded_key) {
            extracted.push(KeyValueEntry {
                key: decoded_key,
                value: decode_query_component(raw_val),
                enabled: true,
                secret: true,
                description: String::new(),
            });
        } else {
            kept_pairs.push(part);
        }
    }

    let mut rebuilt = before_query.to_owned();

    if !kept_pairs.is_empty() {
        rebuilt.push('?');
        rebuilt.push_str(&kept_pairs.join("&"));
    }
    if let Some(fragment) = fragment_suffix {
        rebuilt.push('#');
        rebuilt.push_str(fragment);
    }

    Ok((rebuilt, extracted))
}

/// Defense-in-depth scrubbing of a stored URL template before returning it to
/// an agent tool or `@` mention.
pub(crate) fn redact_url_template_for_agent(raw: &str) -> String {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return String::new();
    }

    let without_userinfo = if let Some(scheme_end) = trimmed.find("://") {
        let after_scheme = &trimmed[scheme_end + 3..];
        let authority_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        let authority = &after_scheme[..authority_end];

        if let Some(at_pos) = authority.rfind('@') {
            format!(
                "{}://{}@{}",
                &trimmed[..scheme_end],
                REDACTED,
                &after_scheme[at_pos + 1..]
            )
        } else {
            trimmed.to_owned()
        }
    } else {
        trimmed.to_owned()
    };

    let Some((before_query, raw_query)) = without_userinfo.split_once('?') else {
        return without_userinfo;
    };
    let (query_part, fragment_suffix) = match raw_query.split_once('#') {
        Some((q, f)) => (q, Some(f)),
        None => (raw_query, None),
    };
    let mut pairs = Vec::new();

    for part in query_part.split('&') {
        if part.is_empty() {
            continue;
        }

        let (raw_key, _) = part.split_once('=').unwrap_or((part, ""));
        let decoded_key = decode_query_component(raw_key);

        if is_secret_name(&decoded_key) {
            pairs.push(format!("{raw_key}={REDACTED}"));
        } else {
            pairs.push(part.to_owned());
        }
    }

    let mut out = before_query.to_owned();

    if !pairs.is_empty() {
        out.push('?');
        out.push_str(&pairs.join("&"));
    }
    if let Some(fragment) = fragment_suffix {
        out.push('#');
        out.push_str(fragment);
    }

    out
}

fn url_embeds_userinfo(raw: &str) -> bool {
    if let Ok(parsed) = Url::parse(raw)
        && (!parsed.username().is_empty() || parsed.password().is_some())
    {
        return true;
    }

    if let Some(scheme_end) = raw.find("://") {
        let after_scheme = &raw[scheme_end + 3..];
        let authority_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        let authority = &after_scheme[..authority_end];

        if authority.contains('@') {
            return true;
        }
    }

    false
}

fn decode_query_component(raw: &str) -> String {
    url::form_urlencoded::parse(format!("k={raw}").as_bytes())
        .next()
        .map_or_else(|| raw.to_owned(), |(_, val)| val.into_owned())
}

fn preview_text(text: &str) -> String {
    const MAX_PREVIEW: usize = 512;

    if text.len() <= MAX_PREVIEW {
        return text.to_owned();
    }

    let end = text.floor_char_boundary(MAX_PREVIEW);
    format!("{}...", &text[..end])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use mannie_plugin_sdk::contracts::UtcMillis;

    use crate::model::{
        ApiKeyLocation, AuthConfig, BodyConfig, CollectionView, EnvironmentView, HttpMethod,
        KeyValueEntry, NetworkError, Protocol, RequestView, VariableEntry,
    };

    use super::{
        OriginPolicy, ResolveError, RuntimeOverrides, redact_url_template_for_agent,
        resolve_http_request, sanitize_saved_url,
    };

    fn sample_collection() -> CollectionView {
        CollectionView {
            id: "col-1".to_owned(),
            name: "Billing".to_owned(),
            description: String::new(),
            protocol: Protocol::Http,
            base_url: "{{host}}/v1".to_owned(),
            enabled: true,
            active_environment_id: None,
            auto_cookies: true,
            headers: vec![],
            query_params: vec![],
            auth: AuthConfig::ApiKey {
                name: "api_key".to_owned(),
                value: "{{billing_key}}".to_owned(),
                location: ApiKeyLocation::Query,
            },
            variables: vec![
                VariableEntry {
                    key: "host".to_owned(),
                    value: "https://staging.example.com".to_owned(),
                    enabled: true,
                    secret: false,
                    description: String::new(),
                },
                VariableEntry {
                    key: "billing_key".to_owned(),
                    value: "sk_live_secret_value".to_owned(),
                    enabled: true,
                    secret: true,
                    description: String::new(),
                },
            ],
            folders: vec![],
            environments: vec![],
            requests: vec![],
            created_at: UtcMillis::from_millis(0),
            updated_at: UtcMillis::from_millis(0),
        }
    }

    fn sample_request() -> RequestView {
        RequestView {
            id: "req-1".to_owned(),
            collection_id: "col-1".to_owned(),
            folder_id: None,
            name: "Get Invoice".to_owned(),
            description: String::new(),
            protocol: Protocol::Http,
            method: HttpMethod::Get,
            url_template: "/invoices/{id}".to_owned(),
            path_params: vec![KeyValueEntry {
                key: "id".to_owned(),
                value: "inv/42".to_owned(),
                enabled: true,
                secret: false,
                description: String::new(),
            }],
            query_params: vec![],
            headers: vec![],
            auth: AuthConfig::Inherit,
            body: BodyConfig::None,
            timeout_seconds: 30,
            use_cookies: true,
            sort_order: 0,
            created_at: UtcMillis::from_millis(0),
            updated_at: UtcMillis::from_millis(0),
        }
    }

    #[test]
    fn resolves_variables_encodes_path_and_redacts_secret_query_params() {
        let collection = sample_collection();
        let request = sample_request();
        let prod = EnvironmentView {
            id: "env-prod".to_owned(),
            collection_id: "col-1".to_owned(),
            name: "Production".to_owned(),
            variables: vec![VariableEntry {
                key: "host".to_owned(),
                value: "https://api.example.com".to_owned(),
                enabled: true,
                secret: false,
                description: String::new(),
            }],
            created_at: UtcMillis::from_millis(0),
            updated_at: UtcMillis::from_millis(0),
        };

        let resolved = resolve_http_request(
            &collection,
            &request,
            Some(&prod),
            &[],
            &RuntimeOverrides::default(),
            OriginPolicy::OwnerWorkbench,
        )
        .unwrap();

        assert_eq!(resolved.origin, "https://api.example.com");
        assert_eq!(
            resolved.real_url.as_str(),
            "https://api.example.com/v1/invoices/inv%2F42?api_key=sk_live_secret_value"
        );
        assert!(resolved.redacted_url.contains("inv%2F42"));
        assert!(!resolved.redacted_url.contains("sk_live_secret_value"));
        assert!(!resolved.summary_json.contains("sk_live_secret_value"));
        assert!(resolved.uses_credentials);
    }

    #[test]
    fn refuses_userinfo_and_secret_origin() {
        let mut collection = sample_collection();
        collection.base_url = "https://user:pass@api.example.com".to_owned();
        let request = sample_request();
        assert_eq!(
            resolve_http_request(
                &collection,
                &request,
                None,
                &[],
                &RuntimeOverrides::default(),
                OriginPolicy::OwnerWorkbench,
            )
            .unwrap_err(),
            ResolveError::InvalidUrl
        );

        collection.base_url = "https://{{secret_host}}".to_owned();
        collection.variables.push(VariableEntry {
            key: "secret_host".to_owned(),
            value: "internal.example.com".to_owned(),
            enabled: true,
            secret: true,
            description: String::new(),
        });
        assert_eq!(
            resolve_http_request(
                &collection,
                &request,
                None,
                &[],
                &RuntimeOverrides::default(),
                OriginPolicy::OwnerWorkbench,
            )
            .unwrap_err(),
            ResolveError::SecretInOrigin
        );

        let overrides = RuntimeOverrides {
            path_params: BTreeMap::from([("id".to_owned(), "bad\nid".to_owned())]),
            ..RuntimeOverrides::default()
        };
        collection.base_url = "https://api.example.com".to_owned();
        assert_eq!(
            resolve_http_request(
                &collection,
                &request,
                None,
                &[],
                &overrides,
                OriginPolicy::OwnerWorkbench,
            )
            .unwrap_err(),
            ResolveError::InvalidPathParam
        );
    }

    #[test]
    fn agent_cannot_redirect_request_origin_via_variable_overrides() {
        let collection = sample_collection();
        let request = sample_request();
        let redirect = RuntimeOverrides {
            variables: BTreeMap::from([("host".to_owned(), "https://attacker.example".to_owned())]),
            ..RuntimeOverrides::default()
        };

        assert_eq!(
            resolve_http_request(
                &collection,
                &request,
                None,
                &[],
                &redirect,
                OriginPolicy::AgentBoundToOwnerOrigin,
            )
            .unwrap_err(),
            ResolveError::OriginOverrideRefused
        );

        // Non-origin overrides still work under AgentBoundToOwnerOrigin.
        let safe_override = RuntimeOverrides {
            path_params: BTreeMap::from([("id".to_owned(), "inv-99".to_owned())]),
            ..RuntimeOverrides::default()
        };
        let resolved = resolve_http_request(
            &collection,
            &request,
            None,
            &[],
            &safe_override,
            OriginPolicy::AgentBoundToOwnerOrigin,
        )
        .unwrap();
        assert_eq!(resolved.origin, "https://staging.example.com");
        assert!(resolved.redacted_url.contains("inv-99"));
    }

    #[test]
    fn interpolates_nested_server_variables_and_tracks_secret_provenance() {
        let mut collection = sample_collection();
        collection.base_url = "{{base_url}}".to_owned();
        collection.auth = AuthConfig::None;
        let env = EnvironmentView {
            id: "env-multi".to_owned(),
            collection_id: "col-1".to_owned(),
            name: "Multi".to_owned(),
            variables: vec![
                VariableEntry {
                    key: "base_url".to_owned(),
                    value: "https://{{tenant}}.api.example.com/{{version}}".to_owned(),
                    enabled: true,
                    secret: false,
                    description: String::new(),
                },
                VariableEntry {
                    key: "tenant".to_owned(),
                    value: "acme".to_owned(),
                    enabled: true,
                    secret: false,
                    description: String::new(),
                },
                VariableEntry {
                    key: "version".to_owned(),
                    value: "v2".to_owned(),
                    enabled: true,
                    secret: false,
                    description: String::new(),
                },
            ],
            created_at: UtcMillis::from_millis(0),
            updated_at: UtcMillis::from_millis(0),
        };
        let request = sample_request();

        let resolved = resolve_http_request(
            &collection,
            &request,
            Some(&env),
            &[],
            &RuntimeOverrides::default(),
            OriginPolicy::AgentBoundToOwnerOrigin,
        )
        .unwrap();
        assert_eq!(resolved.origin, "https://acme.api.example.com");
        assert_eq!(
            resolved.real_url.as_str(),
            "https://acme.api.example.com/v2/invoices/inv%2F42"
        );
    }

    #[test]
    fn summary_digest_changes_when_tail_beyond_preview_differs() {
        let collection = sample_collection();
        let mut request = sample_request();
        request.method = HttpMethod::Post;
        let prefix = "x".repeat(600);
        request.body = BodyConfig::Json {
            content: format!("{{\"data\":\"{prefix}A\"}}"),
        };
        let first = resolve_http_request(
            &collection,
            &request,
            None,
            &[],
            &RuntimeOverrides::default(),
            OriginPolicy::AgentBoundToOwnerOrigin,
        )
        .unwrap();

        request.body = BodyConfig::Json {
            content: format!("{{\"data\":\"{prefix}B\"}}"),
        };
        let second = resolve_http_request(
            &collection,
            &request,
            None,
            &[],
            &RuntimeOverrides::default(),
            OriginPolicy::AgentBoundToOwnerOrigin,
        )
        .unwrap();

        assert_ne!(first.summary_json, second.summary_json);
        assert!(first.summary_json.contains("\"sha256\""));
    }

    #[test]
    fn sanitizes_and_redacts_urls_with_userinfo_or_secret_query_params() {
        assert_eq!(
            sanitize_saved_url("https://alice:pw@api.example.com/v1").unwrap_err(),
            NetworkError::InvalidUrl
        );

        let (cleaned, extracted) =
            sanitize_saved_url("https://api.example.com/items?limit=10&api_key=sk_123&token=tok_9")
                .unwrap();
        assert_eq!(cleaned, "https://api.example.com/items?limit=10");
        assert_eq!(extracted.len(), 2);
        assert!(extracted.iter().all(|entry| entry.secret && entry.enabled));
        assert_eq!(extracted[0].key, "api_key");
        assert_eq!(extracted[0].value, "sk_123");
        assert_eq!(extracted[1].key, "token");
        assert_eq!(extracted[1].value, "tok_9");

        let scrubbed = redact_url_template_for_agent(
            "https://alice:pw@api.example.com/items?limit=10&api_key=sk_123",
        );
        assert!(!scrubbed.contains("pw"));
        assert!(!scrubbed.contains("sk_123"));
        assert!(scrubbed.contains("limit=10"));
    }
}
