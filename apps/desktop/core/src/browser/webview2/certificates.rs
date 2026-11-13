//! Saved server certificate exceptions (ADR-0066).

#[cfg(test)]
mod tests;
mod verify;

use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use sha2::{Digest, Sha256};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW,
    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL,
    COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID, COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
    ICoreWebView2, ICoreWebView2_14, ICoreWebView2ServerCertificateErrorDetectedEventArgs,
};
use webview2_com::{ServerCertificateErrorDetectedEventHandler, take_pwstr};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::System::Threading::{
    CloseThreadpoolTimer, CreateThreadpoolTimer, DisassociateCurrentThreadFromCallback,
    PTP_CALLBACK_INSTANCE, PTP_TIMER, SetThreadpoolTimer, WaitForThreadpoolTimerCallbacks,
};
use windows::core::{Interface as _, PWSTR};

use super::super::certificate::{CertificatePin, https_hostname};
use super::{Shared, lock};
use verify::{decode_pem, verified_leaf};

/// WebView2's cache does not expire with the leaf. Reserve time to terminate
/// its processes and close any pooled connections before `NotAfter`.
const SHUTDOWN_MARGIN: Duration = Duration::from_secs(5);
const TICKS_PER_SECOND: u64 = 10_000_000;
/// The saved pins that WebView2 allowed and the earliest cutoff.
#[derive(Default)]
pub(super) struct Certificates {
    allowed: HashSet<CertificatePin>,
    expiry: Option<ExpiryTimer>,
    cutoff: Option<u64>,
}

impl Certificates {
    pub(super) fn allowed(&self, pin: &CertificatePin) -> bool {
        self.allowed.contains(pin)
    }

    pub(super) fn due(&self) -> bool {
        self.cutoff.is_some_and(|cutoff| cutoff <= current_ticks())
    }

    fn arm(&mut self, shared: &Arc<Shared>, not_after: FILETIME) -> windows::core::Result<bool> {
        let cutoff = ticks(not_after).saturating_sub(SHUTDOWN_MARGIN.as_secs() * TICKS_PER_SECOND);
        if cutoff <= current_ticks() || self.due() {
            return Ok(false);
        }
        if self.cutoff.is_some_and(|scheduled| scheduled <= cutoff) {
            return Ok(true);
        }
        if self.expiry.is_none() {
            self.expiry = Some(ExpiryTimer::new(shared)?);
        }
        let Some(timer) = &self.expiry else {
            return Ok(false);
        };
        timer.arm(cutoff);
        self.cutoff = Some(cutoff);
        Ok(true)
    }
}

/// The stable callback context outlives the timer and its pending callbacks.
struct ExpiryTimer {
    timer: PTP_TIMER,
    _session: Box<Weak<Shared>>,
}

impl ExpiryTimer {
    fn new(shared: &Arc<Shared>) -> windows::core::Result<Self> {
        let session = Box::new(Arc::downgrade(shared));
        let context = std::ptr::from_ref(session.as_ref()).cast_mut().cast();
        // SAFETY: the boxed context stays at the same address through teardown.
        let timer = unsafe { CreateThreadpoolTimer(Some(expire), Some(context), None) }?;
        Ok(Self {
            timer,
            _session: session,
        })
    }

    fn arm(&self, cutoff: u64) {
        let due = FILETIME {
            dwLowDateTime: u32::try_from(cutoff & u64::from(u32::MAX)).expect("masked to 32 bits"),
            dwHighDateTime: u32::try_from(cutoff >> 32).expect("shifted to 32 bits"),
        };
        // SAFETY: the timer is live. Windows copies the absolute UTC due time.
        unsafe { SetThreadpoolTimer(self.timer, Some(&raw const due), 0, Some(0)) };
    }
}

impl Drop for ExpiryTimer {
    fn drop(&mut self) {
        // SAFETY: this owns the timer. Cancel new callbacks before waiting.
        unsafe { SetThreadpoolTimer(self.timer, None, 0, None) };
        // SAFETY: the live timer's callback disassociates before touching Shared.
        unsafe { WaitForThreadpoolTimerCallbacks(self.timer, true) };
        // SAFETY: callbacks can no longer use the timer or boxed context.
        unsafe { CloseThreadpoolTimer(self.timer) };
    }
}

unsafe extern "system" fn expire(
    instance: PTP_CALLBACK_INSTANCE,
    context: *mut c_void,
    _timer: PTP_TIMER,
) {
    // SAFETY: the timer owns this boxed Weak until after this callback
    // disassociates. Take an independent reference before disassociating.
    let shared = unsafe { &*context.cast::<Weak<Shared>>() }.upgrade();
    // SAFETY: this is the instance of the current timer callback. No later
    // code touches the timer or context, so teardown can proceed.
    unsafe { DisassociateCurrentThreadFromCallback(instance) };
    if let Some(shared) = shared {
        expire_session(&shared);
    }
}

pub(super) fn expire_session(shared: &Shared) {
    if shared.ended.load(Ordering::Acquire)
        || shared.certificate_expired.swap(true, Ordering::AcqRel)
    {
        return;
    }
    // End the browser immediately. Clearing cached TLS decisions would not
    // close established connections. Normal session cleanup follows.
    if let Err(error) = shared.job.terminate() {
        tracing::warn!(%error, "could not terminate a browser session with an expiring certificate");
        shared.terminate_tracked_processes();
    }
    shared.fail();
}

fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

fn current_ticks() -> u64 {
    // SAFETY: the system call takes no arguments and returns a value.
    ticks(unsafe { GetSystemTimeAsFileTime() })
}

/// Install the TLS guard on every tab before it can navigate.
pub(super) fn guard(
    shared: &Arc<Shared>,
    webview: &ICoreWebView2,
    rejected: impl Fn(&str) + 'static,
) -> windows::core::Result<()> {
    let api: ICoreWebView2_14 = webview.cast()?;
    let session = Arc::clone(shared);
    let handler =
        ServerCertificateErrorDetectedEventHandler::create(Box::new(move |_, arguments| {
            if let Some(arguments) = arguments {
                // SAFETY: the callback arguments are live on this thread.
                unsafe {
                    arguments.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL)
                }?;
                let mut uri = PWSTR::null();
                // SAFETY: the arguments are live and the output is writable.
                unsafe { arguments.RequestUri(&raw mut uri) }?;
                let uri = take_pwstr(uri);
                // Verification errors leave the explicit cancellation in place.
                if !allow_if_trusted(&session, &arguments, &uri).unwrap_or(false) {
                    rejected(&uri);
                }
            }
            Ok(())
        }));
    let mut token = 0;
    // SAFETY: WebView2 retains the handler, and the token is writable.
    unsafe { api.add_ServerCertificateErrorDetected(&handler, &raw mut token) }
}

fn allow_if_trusted(
    shared: &Arc<Shared>,
    arguments: &ICoreWebView2ServerCertificateErrorDetectedEventArgs,
    uri: &str,
) -> windows::core::Result<bool> {
    if shared.stopping() || shared.certificate_deadline_reached() {
        return Ok(false);
    }
    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
    // SAFETY: the arguments are live on this thread and the output is writable.
    unsafe { arguments.ErrorStatus(&raw mut status) }?;
    if status != COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID {
        return Ok(false);
    }
    let Some(hostname) = https_hostname(uri) else {
        return Ok(false);
    };
    let fingerprint = {
        let saved = lock(&shared.saved_certificates);
        saved.as_ref().and_then(|pins| pins.get(&hostname)).copied()
    };
    let Some(fingerprint) = fingerprint else {
        return Ok(false);
    };
    // SAFETY: the arguments are live on this thread.
    let certificate = unsafe { arguments.ServerCertificate() }?;
    let mut pem = PWSTR::null();
    // SAFETY: the certificate is live and the output is writable. take_pwstr frees it.
    unsafe { certificate.ToPemEncoding(&raw mut pem) }?;
    let der = decode_pem(&take_pwstr(pem))?;
    if Sha256::digest(&der)[..] != fingerprint {
        return Ok(false);
    }
    let Some(not_after) = verified_leaf(&der, &hostname, None) else {
        return Ok(false);
    };
    let pin = CertificatePin {
        hostname,
        fingerprint,
    };
    // Forget takes these locks in the same order. It either sees the allowance
    // here or prevents it before WebView2 can cache it.
    let saved = lock(&shared.saved_certificates);
    let mut certificates = lock(&shared.certificates);
    if shared.stopping()
        || shared.certificate_expired.load(Ordering::Acquire)
        || saved.as_ref().and_then(|pins| pins.get(&pin.hostname)) != Some(&pin.fingerprint)
        || !certificates.arm(shared, not_after)?
    {
        return Ok(false);
    }
    // Keep the allowance recorded even if WebView2 reports an error setting it.
    certificates.allowed.insert(pin);
    // SAFETY: the live callback arguments accept this action.
    unsafe { arguments.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW) }?;
    Ok(true)
}
