//! Self-management provider for the Browser plugin (ADR-0081).
//!
//! Exposes saved site credential metadata (never secret values), saved TLS
//! certificate pins, and typed actions for adding or removing site credentials
//! and certificate pins under the master's always-ask `Access::AppConfig`
//! policy.

use std::sync::Arc;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{ConfigFieldPreview, ConfigSecretField};
use mannie_plugin_sdk::points::management::{
    ActionResult, ExecuteActionRequest, ManagementActionSpec, ManagementError, ManagementGuide,
    ManagementProvider, ManagementSnapshot, PrepareActionRequest, PreparedAction, SecretFieldSpec,
};
use mannie_plugin_sdk::services::browser::{BrowserApi, BrowserError};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Plugin ID for the Browser management provider.
pub(crate) const BROWSER: &str = "browser";

const ACTION_ADD_SITE_CREDENTIAL: &str = "add_site_credential";
const ACTION_REMOVE_SITE_CREDENTIAL: &str = "remove_site_credential";
const ACTION_SAVE_CERTIFICATE: &str = "save_certificate";
const ACTION_FORGET_CERTIFICATE: &str = "forget_certificate";
const SECRET_VALUE: &str = "value";

/// Management provider for the Browser plugin.
#[derive(Debug, Clone)]
pub(crate) struct BrowserManagement {
    browser: Arc<dyn BrowserApi>,
}

impl BrowserManagement {
    /// Creates a management provider backed by the desktop browser service.
    pub(crate) fn new(browser: Arc<dyn BrowserApi>) -> Self {
        Self { browser }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddSiteCredentialArgs {
    origin: String,
    label: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoveSiteCredentialArgs {
    reference: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedCertificateArgs {
    hostname: String,
    fingerprint: String,
}

fn map_browser_error(error: BrowserError) -> ManagementError {
    match error {
        BrowserError::NotFound => ManagementError::not_found(error.to_string()),
        BrowserError::CredentialAlreadyExists | BrowserError::CertificateSavedConflict => {
            ManagementError::conflict(error.to_string())
        }
        BrowserError::InvalidBrowserInput => ManagementError::invalid_arguments(error.to_string()),
        other => ManagementError::new(format!("browser.{}", other.as_code()), other.to_string()),
    }
}

fn validate_non_empty_bounded(
    value: &str,
    limit: usize,
    field: &str,
) -> Result<String, ManagementError> {
    let trimmed = value.trim();
    let valid = !trimmed.is_empty() && value.len() <= limit;
    if !valid {
        return Err(ManagementError::invalid_field(
            field,
            format!("{field} must be non-empty and at most {limit} bytes"),
        ));
    }
    Ok(trimmed.to_owned())
}

fn guide_actions() -> Vec<ManagementActionSpec> {
    vec![
        ManagementActionSpec {
            name: ACTION_ADD_SITE_CREDENTIAL.to_owned(),
            description: "Save a protected site credential for an exact HTTP or HTTPS origin in \
                the OS credential store."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "origin": {
                        "type": "string",
                        "description": "Exact HTTP or HTTPS origin (for example https://example.com)."
                    },
                    "label": {
                        "type": "string",
                        "description": "Owner-facing label identifying this credential at that origin."
                    }
                },
                "required": ["origin", "label"],
                "additionalProperties": false
            }),
            secret_fields: vec![SecretFieldSpec {
                key: SECRET_VALUE.to_owned(),
                label: "Credential value".to_owned(),
                description: "Secret credential value stored in the OS credential store."
                    .to_owned(),
                required: true,
            }],
        },
        ManagementActionSpec {
            name: ACTION_REMOVE_SITE_CREDENTIAL.to_owned(),
            description: "Remove a saved Browser site credential by its opaque reference and \
                revoke its active grants."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "reference": {
                        "type": "string",
                        "description": "36-character opaque credential reference returned by inspect."
                    }
                },
                "required": ["reference"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_SAVE_CERTIFICATE.to_owned(),
            description: "Save an owner-confirmed TLS leaf certificate SHA-256 pin for a hostname \
                across Browser sessions (supports self-signed, private-CA, hostname-mismatched \
                and expired certificates, and multiple pins per hostname)."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "hostname": {
                        "type": "string",
                        "description": "Hostname, IP address, or pasted HTTPS URL to trust this certificate for."
                    },
                    "fingerprint": {
                        "type": "string",
                        "description": "SHA-256 leaf certificate fingerprint (plain hex, colon-, space-, or hyphen-separated, or prefixed)."
                    }
                },
                "required": ["hostname", "fingerprint"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
        ManagementActionSpec {
            name: ACTION_FORGET_CERTIFICATE.to_owned(),
            description: "Forget a saved TLS certificate pin and close any Browser sessions that \
                trusted it."
                .to_owned(),
            security_sensitive: true,
            parameters: json!({
                "type": "object",
                "properties": {
                    "hostname": {
                        "type": "string",
                        "description": "Hostname or IP address of the saved certificate pin."
                    },
                    "fingerprint": {
                        "type": "string",
                        "description": "SHA-256 fingerprint of the saved certificate pin."
                    }
                },
                "required": ["hostname", "fingerprint"],
                "additionalProperties": false
            }),
            secret_fields: Vec::new(),
        },
    ]
}

impl ManagementProvider for BrowserManagement {
    fn guide(&self) -> ManagementGuide {
        ManagementGuide {
            summary: "Provides an embedded browser pane, conversation browser tools (navigation, \
                page observation, interaction, and owner handoff), saved TLS certificate pins, and \
                saved site credentials scoped to exact canonical HTTP or HTTPS origins backed by \
                the desktop WebView2 engine on Windows. `inspect` returns all saved \
                `siteCredentials` (`reference`, `origin`, `label`) without secret values. To enter \
                a saved credential on its matching origin, pass its 36-character UUID `reference` \
                (not the label) to `browser_interact` with `action: {\"kind\":\"fill\",\"reference\":\
                \"<element_ref>\",\"value\":{\"kind\":\"credential\",\"reference\":\"<uuid>\"}}`."
                .to_owned(),
            storage: "Stores saved site credential metadata and certificate pins in the desktop \
                host's browser catalog in SQLite, while actual site credential secret values live \
                only in the master's OS credential store and never reach the model or webview."
                .to_owned(),
            troubleshooting: "Browser requires Microsoft Edge WebView2 on Windows and does not \
                start on unsupported platforms. If navigation fails with certificate_rejected, \
                check Mannie's log for the exact rejection reason and presented fingerprint, or \
                inspect and trust the certificate in the Browser pane or via save_certificate. \
                Multiple certificate pins may be saved for the same hostname. Password fields \
                reject plain-text fills (`value.kind = \"text\"`): use `value.kind = \"credential\"` \
                with the 36-character UUID from `browser_observe`'s `credentials` array or \
                `inspect`. Saved credentials match exact canonical origin (`scheme://host[:port]`)."
                .to_owned(),
            actions: guide_actions(),
        }
    }

    fn inspect(
        &self,
        query: Option<String>,
    ) -> BoxFuture<'static, Result<ManagementSnapshot, ManagementError>> {
        let browser = Arc::clone(&self.browser);

        Box::pin(async move {
            if let Some(query) = query
                && !query.trim().is_empty()
            {
                return Err(ManagementError::invalid_field(
                    "query",
                    format!("browser does not support inspect query {query:?}"),
                ));
            }

            let supported = browser.supported();
            let credentials = match browser.site_credentials().await {
                Ok(list) => list,
                Err(BrowserError::BrowserIncompatible) => Vec::new(),
                Err(error) => return Err(map_browser_error(error)),
            };
            let certificates = match browser.saved_certificates().await {
                Ok(list) => list,
                Err(BrowserError::BrowserIncompatible) => Vec::new(),
                Err(error) => return Err(map_browser_error(error)),
            };

            let summary = format!(
                "Browser plugin is on (supported: {supported}, {} saved site credential(s), {} saved certificate pin(s)).",
                credentials.len(),
                certificates.len()
            );

            let data = json!({
                "supported": supported,
                "siteCredentials": credentials,
                "savedCertificates": certificates,
            });

            Ok(ManagementSnapshot::new(summary, data))
        })
    }

    fn prepare_action(
        &self,
        request: PrepareActionRequest,
    ) -> BoxFuture<'static, Result<PreparedAction, ManagementError>> {
        Box::pin(async move {
            match request.action.as_str() {
                ACTION_ADD_SITE_CREDENTIAL => {
                    let provided_in_chat = request.has_provided_secret(SECRET_VALUE);
                    let args: AddSiteCredentialArgs = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;

                    let origin = validate_non_empty_bounded(&args.origin, 512, "origin")?;
                    let label = validate_non_empty_bounded(&args.label, 80, "label")?;
                    let canonical = AddSiteCredentialArgs {
                        origin: origin.clone(),
                        label: label.clone(),
                    };

                    let canonical_arguments = serde_json::to_value(&canonical)
                        .map_err(|error| ManagementError::failed(error.to_string()))?;

                    Ok(PreparedAction {
                        title: format!("Add browser credential {label}"),
                        fields: vec![
                            ConfigFieldPreview {
                                label: "Origin".to_owned(),
                                value: origin,
                            },
                            ConfigFieldPreview {
                                label: "Label".to_owned(),
                                value: label,
                            },
                        ],
                        secret_fields: vec![ConfigSecretField {
                            key: SECRET_VALUE.to_owned(),
                            label: "Credential value".to_owned(),
                            required: true,
                            provided_in_chat,
                        }],
                        security_sensitive: true,
                        canonical_arguments,
                    })
                }
                ACTION_REMOVE_SITE_CREDENTIAL => {
                    let args: RemoveSiteCredentialArgs = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    if args.reference.len() != 36 {
                        return Err(ManagementError::invalid_field(
                            "reference",
                            "reference must be a 36-character credential reference",
                        ));
                    }

                    let canonical_arguments = serde_json::to_value(&args)
                        .map_err(|error| ManagementError::failed(error.to_string()))?;

                    Ok(PreparedAction {
                        title: "Remove browser site credential".to_owned(),
                        fields: vec![ConfigFieldPreview {
                            label: "Reference".to_owned(),
                            value: args.reference,
                        }],
                        secret_fields: Vec::new(),
                        security_sensitive: true,
                        canonical_arguments,
                    })
                }
                ACTION_SAVE_CERTIFICATE | ACTION_FORGET_CERTIFICATE => {
                    let args: SavedCertificateArgs = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    let hostname = validate_non_empty_bounded(&args.hostname, 1024, "hostname")?;
                    let fingerprint =
                        validate_non_empty_bounded(&args.fingerprint, 160, "fingerprint")?;
                    let canonical = SavedCertificateArgs {
                        hostname: hostname.clone(),
                        fingerprint: fingerprint.clone(),
                    };

                    let title = if request.action == ACTION_SAVE_CERTIFICATE {
                        format!("Save browser certificate pin for {hostname}")
                    } else {
                        format!("Forget browser certificate pin for {hostname}")
                    };
                    let canonical_arguments = serde_json::to_value(&canonical)
                        .map_err(|error| ManagementError::failed(error.to_string()))?;

                    Ok(PreparedAction {
                        title,
                        fields: vec![
                            ConfigFieldPreview {
                                label: "Hostname".to_owned(),
                                value: hostname,
                            },
                            ConfigFieldPreview {
                                label: "Fingerprint".to_owned(),
                                value: fingerprint,
                            },
                        ],
                        secret_fields: Vec::new(),
                        security_sensitive: true,
                        canonical_arguments,
                    })
                }
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }

    fn execute_action(
        &self,
        mut request: ExecuteActionRequest,
    ) -> BoxFuture<'static, Result<ActionResult, ManagementError>> {
        let browser = Arc::clone(&self.browser);

        Box::pin(async move {
            match request.action.as_str() {
                ACTION_ADD_SITE_CREDENTIAL => {
                    let args: AddSiteCredentialArgs = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    let value = request.secrets.remove(SECRET_VALUE).ok_or_else(|| {
                        ManagementError::invalid_field(
                            SECRET_VALUE,
                            "a credential value is required",
                        )
                    })?;
                    let valid_value = !value.is_empty() && value.expose().len() <= 8192;
                    if !valid_value {
                        return Err(ManagementError::invalid_field(
                            SECRET_VALUE,
                            "credential value must be 1 to 8192 bytes",
                        ));
                    }

                    let saved = browser
                        .add_site_credential(args.origin, args.label, value)
                        .await
                        .map_err(map_browser_error)?;
                    let message = format!(
                        "Saved browser credential {} for {}.",
                        saved.label, saved.origin
                    );
                    let data = serde_json::to_value(&saved)
                        .map_err(|error| ManagementError::failed(error.to_string()))?;

                    Ok(ActionResult::new(message).with_data(data))
                }
                ACTION_REMOVE_SITE_CREDENTIAL => {
                    let args: RemoveSiteCredentialArgs = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    browser
                        .remove_site_credential(args.reference.clone())
                        .await
                        .map_err(map_browser_error)?;

                    Ok(ActionResult::new("Removed browser site credential.")
                        .with_data(json!({ "reference": args.reference })))
                }
                ACTION_SAVE_CERTIFICATE => {
                    let args: SavedCertificateArgs = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    browser
                        .save_certificate(args.hostname.clone(), args.fingerprint.clone())
                        .await
                        .map_err(map_browser_error)?;

                    Ok(ActionResult::new(format!(
                        "Saved browser certificate pin for {}.",
                        args.hostname
                    ))
                    .with_data(json!({
                        "hostname": args.hostname,
                        "fingerprint": args.fingerprint,
                    })))
                }
                ACTION_FORGET_CERTIFICATE => {
                    let args: SavedCertificateArgs = serde_json::from_value(request.arguments)
                        .map_err(|error| ManagementError::invalid_arguments(error.to_string()))?;
                    browser
                        .forget_certificate(args.hostname.clone(), args.fingerprint.clone())
                        .await
                        .map_err(map_browser_error)?;

                    Ok(ActionResult::new(format!(
                        "Forgot browser certificate pin for {}.",
                        args.hostname
                    ))
                    .with_data(json!({
                        "hostname": args.hostname,
                        "fingerprint": args.fingerprint,
                    })))
                }
                other => Err(ManagementError::unknown_action(other)),
            }
        })
    }
}
