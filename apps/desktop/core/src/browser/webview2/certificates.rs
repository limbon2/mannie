//! Saved server certificate pinning and owner inspection metadata (ADR-0082).

#[cfg(test)]
mod tests;
mod verify;

use std::collections::HashSet;
use std::sync::Arc;

use mannie_tools::browser::ChromeRejectedCertificate;
use mannie_types::clock::{Clock, SystemClock};
use sha2::{Digest, Sha256};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW,
    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL, COREWEBVIEW2_WEB_ERROR_STATUS,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED, COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
    ICoreWebView2, ICoreWebView2_14, ICoreWebView2ServerCertificateErrorDetectedEventArgs,
};
use webview2_com::{ServerCertificateErrorDetectedEventHandler, take_pwstr};
use windows::core::{Interface as _, PWSTR};

use super::super::certificate::{CertificatePin, fingerprint_hex, tls_hostname};
use super::{Shared, lock};
use verify::{decode_pem, parse_leaf};

/// The saved pins that WebView2 allowed during this session, tracked so
/// forgetting a pin can immediately end sessions with cached allowances.
#[derive(Default)]
pub(super) struct Certificates {
    allowed: HashSet<CertificatePin>,
}

impl Certificates {
    pub(super) fn allowed(&self, pin: &CertificatePin) -> bool {
        self.allowed.contains(pin)
    }
}

enum Decision {
    Allowed,
    Rejected(Option<Box<ChromeRejectedCertificate>>),
}

/// Maps a WebView2 certificate error status to a stable reason code and
/// whether an owner-saved leaf SHA-256 pin may allow it (ADR-0082).
fn classify_status(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> (&'static str, bool) {
    match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID => ("untrusted_authority", true),
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT => {
            ("hostname_mismatch", true)
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED => ("expired", true),
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED => ("revoked", false),
        _ => ("invalid", false),
    }
}

fn current_unix_seconds() -> i64 {
    SystemClock.now().as_millis() / 1_000
}

fn log_rejection(info: &ChromeRejectedCertificate, reject_reason: &str) {
    tracing::warn!(
        hostname = %info.hostname,
        presented_fingerprint = %info.fingerprint,
        status = %info.reason,
        reject_reason,
        subject = %info.subject,
        issuer = %info.issuer,
        expired = info.expired,
        "rejected a Browser server certificate"
    );
}

/// Install the TLS guard on every tab before it can navigate.
pub(super) fn guard(
    shared: &Arc<Shared>,
    webview: &ICoreWebView2,
    rejected: impl Fn(&str, Option<ChromeRejectedCertificate>) + 'static,
) -> windows::core::Result<()> {
    let api: ICoreWebView2_14 = webview.cast()?;
    let session = Arc::clone(shared);
    let handler = ServerCertificateErrorDetectedEventHandler::create(Box::new(
        move |_, arguments| {
            if let Some(arguments) = arguments {
                // SAFETY: the callback arguments are live on this thread.
                unsafe {
                    arguments.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL)
                }?;
                let mut uri = PWSTR::null();
                // SAFETY: the arguments are live and the output is writable.
                unsafe { arguments.RequestUri(&raw mut uri) }?;
                let uri = take_pwstr(uri);
                // Any callback error leaves the explicit cancellation in place.
                match evaluate_certificate(&session, &arguments, &uri) {
                    Ok(Decision::Allowed) => {}
                    Ok(Decision::Rejected(info)) => rejected(&uri, info.map(|boxed| *boxed)),
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "could not inspect a Browser server certificate; keeping request cancelled"
                        );
                        rejected(&uri, None);
                    }
                }
            }
            Ok(())
        },
    ));
    let mut token = 0;
    // SAFETY: WebView2 retains the handler, and the token is writable.
    unsafe { api.add_ServerCertificateErrorDetected(&handler, &raw mut token) }
}

fn evaluate_certificate(
    shared: &Arc<Shared>,
    arguments: &ICoreWebView2ServerCertificateErrorDetectedEventArgs,
    uri: &str,
) -> windows::core::Result<Decision> {
    if shared.stopping() {
        return Ok(Decision::Rejected(None));
    }

    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
    // SAFETY: the arguments are live on this thread and the output is writable.
    unsafe { arguments.ErrorStatus(&raw mut status) }?;
    let (reason, pinnable) = classify_status(status);

    let Some(hostname) = tls_hostname(uri) else {
        tracing::warn!(
            status = reason,
            reject_reason = "unsupported_scheme",
            "rejected a Browser server certificate"
        );
        return Ok(Decision::Rejected(None));
    };

    // SAFETY: the arguments are live on this thread.
    let certificate = unsafe { arguments.ServerCertificate() }?;
    let mut pem = PWSTR::null();
    // SAFETY: the certificate is live and the output is writable. take_pwstr frees it.
    unsafe { certificate.ToPemEncoding(&raw mut pem) }?;
    let der = decode_pem(&take_pwstr(pem))?;
    let fingerprint: [u8; 32] = Sha256::digest(&der).into();
    let fingerprint_text = fingerprint_hex(&fingerprint);

    let Some(parsed) = parse_leaf(&der) else {
        tracing::warn!(
            hostname = %hostname,
            presented_fingerprint = %fingerprint_text,
            status = reason,
            reject_reason = "malformed_certificate",
            "rejected a Browser server certificate"
        );
        return Ok(Decision::Rejected(None));
    };

    let expired = parsed.is_expired(current_unix_seconds());
    let info = ChromeRejectedCertificate {
        hostname: hostname.clone(),
        fingerprint: fingerprint_text.clone(),
        subject: parsed.subject,
        issuer: parsed.issuer,
        sans: parsed.sans,
        not_before: parsed.not_before_iso,
        not_after: parsed.not_after_iso,
        expired,
        self_signed: parsed.self_signed,
        reason: reason.to_owned(),
        url: uri.to_owned(),
    };

    if !pinnable {
        let reject_reason = if status == COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED {
            "certificate_revoked"
        } else {
            "unsupported_status"
        };
        log_rejection(&info, reject_reason);
        return Ok(Decision::Rejected(Some(Box::new(info))));
    }

    let pin = CertificatePin {
        hostname: hostname.clone(),
        fingerprint,
    };
    // Forget takes these locks in the same order. It either sees the allowance
    // here or prevents it before WebView2 can cache it.
    let saved = lock(&shared.saved_certificates);
    let mut certificates = lock(&shared.certificates);
    if shared.stopping() {
        return Ok(Decision::Rejected(Some(Box::new(info))));
    }

    let host_pins = saved.as_ref().and_then(|pins| pins.get(&pin.hostname));
    let matched = host_pins.is_some_and(|fingerprints| fingerprints.contains(&pin.fingerprint));
    if !matched {
        let reject_reason = if host_pins.is_some_and(|fingerprints| !fingerprints.is_empty()) {
            "fingerprint_mismatch"
        } else {
            "unpinned_host"
        };
        log_rejection(&info, reject_reason);
        return Ok(Decision::Rejected(Some(Box::new(info))));
    }

    // Keep the allowance recorded even if WebView2 reports an error setting it.
    certificates.allowed.insert(pin);
    // SAFETY: the live callback arguments accept this action.
    unsafe { arguments.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW) }?;
    tracing::info!(
        hostname = %hostname,
        fingerprint = %fingerprint_text,
        status = reason,
        expired,
        "allowed a pinned Browser server certificate"
    );
    Ok(Decision::Allowed)
}
