//! Canonical certificate identities for owner-confirmed session exceptions.

#[cfg(windows)]
use std::collections::HashMap;
use std::net::Ipv6Addr;
use std::sync::Arc;
#[cfg(windows)]
use std::sync::Mutex;

use mannie_plugin_sdk::contracts::ConversationId;
#[cfg(windows)]
use mannie_plugin_sdk::services::browser::BrowserCertificateException;
use mannie_plugin_sdk::services::browser::BrowserError;
use mannie_store::Store;
use url::Host;
#[cfg(windows)]
use url::Url;

/// Loaded from the Store before the first native session starts. The TLS
/// callback holds the lock through its final check and allowance.
#[cfg(windows)]
pub(super) type SavedCertificates = Arc<Mutex<Option<HashMap<String, [u8; 32]>>>>;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct CertificatePin {
    pub hostname: String,
    pub fingerprint: [u8; 32],
}

impl CertificatePin {
    pub(super) fn parse(hostname: &str, fingerprint: &str) -> Result<Self, BrowserError> {
        let hostname = bare_hostname(hostname).ok_or(BrowserError::InvalidBrowserInput)?;
        let fingerprint =
            parse_fingerprint(fingerprint).ok_or(BrowserError::InvalidBrowserInput)?;
        Ok(Self {
            hostname,
            fingerprint,
        })
    }

    pub(super) fn fingerprint_text(&self) -> String {
        use std::fmt::Write as _;

        let mut text = String::with_capacity(64);
        for byte in self.fingerprint {
            write!(&mut text, "{byte:02x}").expect("writing to a string succeeds");
        }
        text
    }

    #[cfg(windows)]
    pub(super) fn descriptor(&self) -> BrowserCertificateException {
        BrowserCertificateException {
            hostname: self.hostname.clone(),
            fingerprint: self.fingerprint_text(),
        }
    }

    pub(super) async fn record_confirmation(
        &self,
        store: &Store,
        conversation: &ConversationId,
        session_id: &str,
    ) -> Result<(), BrowserError> {
        store
            .record_browser_certificate_confirmation(
                conversation,
                session_id,
                &self.hostname,
                &self.fingerprint,
            )
            .await
            .map_err(|error| {
                tracing::warn!(
                    error.code = error.code(),
                    "could not record a Browser certificate confirmation"
                );
                BrowserError::CertificateJournalFailed
            })
    }
}

#[cfg(windows)]
pub(super) fn store_failure(error: &mannie_store::StoreError) -> BrowserError {
    tracing::warn!(
        error.code = error.code(),
        "could not access saved Browser certificates"
    );
    BrowserError::CertificateStoreFailed
}

fn bare_hostname(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 1024
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
        || value.contains(['/', '\\', '@', '?', '#', '%'])
    {
        return None;
    }
    if let Ok(address) = value.parse::<Ipv6Addr>() {
        return Some(address.to_string());
    }
    Host::parse(value).ok().and_then(|host| canonical(&host))
}

fn canonical<T: AsRef<str>>(host: &Host<T>) -> Option<String> {
    match host {
        Host::Domain(name) => {
            let name = name.as_ref().strip_suffix('.').unwrap_or(name.as_ref());
            let valid = name.len() <= 253
                && name.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                });
            valid.then(|| name.to_ascii_lowercase())
        }
        Host::Ipv4(address) => Some(address.to_string()),
        Host::Ipv6(address) => Some(address.to_string()),
    }
}

#[cfg(windows)]
pub(super) fn https_hostname(address: &str) -> Option<String> {
    let url = Url::parse(address).ok()?;
    (url.scheme() == "https").then_some(())?;
    canonical(&url.host()?)
}

fn parse_fingerprint(value: &str) -> Option<[u8; 32]> {
    let bytes = value.trim().as_bytes();
    let stride = match bytes.len() {
        64 => 2,
        95 => 3,
        _ => return None,
    };
    let mut fingerprint = [0; 32];
    for (index, output) in fingerprint.iter_mut().enumerate() {
        let offset = index * stride;
        if stride == 3 && index > 0 && bytes[offset - 1] != b':' {
            return None;
        }
        let high = char::from(bytes[offset]).to_digit(16)?;
        let low = char::from(bytes[offset + 1]).to_digit(16)?;
        *output = u8::try_from(high * 16 + low).ok()?;
    }
    Some(fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_hostnames_are_bare_and_canonical() {
        assert_eq!(
            bare_hostname(" Example.COM. ").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            bare_hostname("BÜCHER.example").as_deref(),
            Some("xn--bcher-kva.example")
        );
        assert_eq!(bare_hostname("127.0.0.1").as_deref(), Some("127.0.0.1"));
        assert_eq!(bare_hostname("[::1]").as_deref(), Some("::1"));
        assert_eq!(bare_hostname("::1").as_deref(), Some("::1"));
        for invalid in [
            "",
            ".",
            "https://localhost",
            "localhost:443",
            "localhost/path",
            "a@b",
            "example.test#fragment",
            "local%68ost",
            "*.example.test",
            "a..b",
            "-a.test",
            "a\tb",
        ] {
            assert!(bare_hostname(invalid).is_none(), "{invalid:?}");
        }
    }

    #[test]
    fn certificate_fingerprints_accept_two_exact_hex_formats() {
        let plain = "AB".repeat(32);
        let separated = ["ab"; 32].join(":");
        assert_eq!(parse_fingerprint(&plain), Some([0xab; 32]));
        assert_eq!(parse_fingerprint(&separated), Some([0xab; 32]));
        assert!(parse_fingerprint(&plain[..62]).is_none());
        assert!(parse_fingerprint(&plain.replace('B', "X")).is_none());
        assert!(parse_fingerprint(&separated.replace(':', "-")).is_none());
    }

    #[tokio::test]
    async fn a_journal_failure_refuses_the_certificate_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let options = mannie_store::StoreOptions::new(
            directory.path().join("backups"),
            Arc::new(mannie_types::clock::SystemClock),
        );
        let store = Store::open(&directory.path().join("test.sqlite"), &options).unwrap();
        let missing = ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();
        let pin = CertificatePin::parse("LOCALHOST", &"AB".repeat(32)).unwrap();
        assert_eq!(pin.fingerprint_text(), "ab".repeat(32));
        assert_eq!(
            pin.record_confirmation(&store, &missing, "session").await,
            Err(BrowserError::CertificateJournalFailed)
        );
    }

    #[cfg(windows)]
    #[test]
    fn certificate_requests_share_a_hostname_across_https_ports() {
        assert_eq!(
            https_hostname("https://LOCALHOST:8443/path").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            https_hostname("https://[::1]:8443/").as_deref(),
            Some("::1")
        );
        assert!(https_hostname("http://localhost:8443").is_none());
    }
}
