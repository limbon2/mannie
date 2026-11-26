//! Canonical certificate identities for owner-confirmed exceptions (ADR-0082).

#[cfg(windows)]
use std::collections::{HashMap, HashSet};
use std::net::Ipv6Addr;
#[cfg(windows)]
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use mannie_plugin_sdk::services::browser::BrowserCertificateException;
use mannie_plugin_sdk::services::browser::BrowserError;
use url::{Host, Url};

/// Loaded from the Store before the first native session starts. The TLS
/// callback holds the lock through its final check and allowance.
#[cfg(windows)]
pub(super) type SavedCertificates = Arc<Mutex<Option<HashMap<String, HashSet<[u8; 32]>>>>>;

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

    #[cfg(any(windows, test))]
    pub(super) fn fingerprint_text(&self) -> String {
        fingerprint_hex(&self.fingerprint)
    }

    #[cfg(windows)]
    pub(super) fn descriptor(&self) -> BrowserCertificateException {
        BrowserCertificateException {
            hostname: self.hostname.clone(),
            fingerprint: self.fingerprint_text(),
        }
    }
}

#[cfg(any(windows, test))]
pub(super) fn fingerprint_hex(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(64);
    for byte in bytes {
        write!(&mut text, "{byte:02x}").expect("writing to a string succeeds");
    }
    text
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
        || value.contains(['\\', '@', '%'])
    {
        return None;
    }

    if value.contains("://") {
        let url = Url::parse(value).ok()?;
        let allowed_scheme = matches!(url.scheme(), "https" | "http" | "wss");
        if !allowed_scheme || !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        return canonical(&url.host()?);
    }

    if let Ok(address) = value.parse::<Ipv6Addr>() {
        return Some(address.to_string());
    }

    if value.starts_with('[') {
        let url = Url::parse(&format!("https://{value}")).ok()?;
        if !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        return canonical(&url.host()?);
    }

    if value.contains([':', '/', '?', '#']) {
        if value.starts_with(['/', '?', '#']) {
            return None;
        }
        let authority = value.split(['/', '?', '#']).next().unwrap_or_default();
        if !authority_port_valid(authority) {
            return None;
        }
        let url = Url::parse(&format!("https://{value}")).ok()?;
        if !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        return canonical(&url.host()?);
    }

    Host::parse(value).ok().and_then(|host| canonical(&host))
}

fn authority_port_valid(authority: &str) -> bool {
    let Some((host, port)) = authority.split_once(':') else {
        return true;
    };
    let has_host = !host.is_empty();
    let has_valid_port = port.parse::<u16>().is_ok_and(|number| number > 0);
    has_host && has_valid_port
}

fn canonical<T: AsRef<str>>(host: &Host<T>) -> Option<String> {
    match host {
        Host::Domain(name) => {
            let name = name.as_ref().strip_suffix('.').unwrap_or(name.as_ref());
            let valid = !name.is_empty()
                && name.len() <= 253
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

/// Extracts the canonical hostname from an `https://` or `wss://` URI.
#[cfg(any(windows, test))]
pub(super) fn tls_hostname(address: &str) -> Option<String> {
    let url = Url::parse(address).ok()?;
    matches!(url.scheme(), "https" | "wss").then_some(())?;
    canonical(&url.host()?)
}

fn strip_fingerprint_prefix(value: &str) -> &str {
    const PREFIXES: [&str; 6] = [
        "sha256 fingerprint=",
        "sha-256 fingerprint=",
        "sha256:",
        "sha-256:",
        "sha256=",
        "sha-256=",
    ];
    let trimmed = value.trim();
    for prefix in PREFIXES {
        if trimmed.len() >= prefix.len()
            && trimmed
                .get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        {
            return trimmed[prefix.len()..].trim();
        }
    }
    trimmed
}

fn parse_fingerprint(value: &str) -> Option<[u8; 32]> {
    let normalized = strip_fingerprint_prefix(value);
    if !normalized.is_ascii() {
        return None;
    }
    let bytes = normalized.as_bytes();
    let (stride, separator) = match bytes.len() {
        64 => (2, None),
        95 => {
            let sep = bytes[2];
            matches!(sep, b':' | b'-' | b' ').then_some((3, Some(sep)))?
        }
        _ => return None,
    };

    let mut fingerprint = [0; 32];
    for (index, output) in fingerprint.iter_mut().enumerate() {
        let offset = index * stride;
        if let Some(sep) = separator
            && index > 0
            && bytes[offset - 1] != sep
        {
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
    fn certificate_hostnames_accept_bare_hosts_ports_and_web_urls() {
        for (input, expected) in [
            (" Example.COM. ", "example.com"),
            ("BÜCHER.example", "xn--bcher-kva.example"),
            ("127.0.0.1", "127.0.0.1"),
            ("[::1]", "::1"),
            ("::1", "::1"),
            ("[::1]:8443", "::1"),
            ("192.168.1.50:8006", "192.168.1.50"),
            ("localhost:8443/login", "localhost"),
            ("https://192.168.1.50:8006/path?q=1#frag", "192.168.1.50"),
            ("HTTPS://LOCALHOST:8443/", "localhost"),
            ("http://pve.local:8006", "pve.local"),
            ("wss://pve.local:8006/api2/ws", "pve.local"),
            ("localhost/path", "localhost"),
        ] {
            assert_eq!(bare_hostname(input).as_deref(), Some(expected), "{input:?}");
        }

        for invalid in [
            "",
            ".",
            "a@b",
            "https://user:pass@example.com",
            "file:///etc/passwd",
            "ftp://example.com",
            "javascript:alert(1)",
            "mailto:owner@example.com",
            "local%68ost",
            "*.example.test",
            "a..b",
            "-a.test",
            "a\tb",
            "/relative/path",
            "localhost:notaport",
            "localhost:0",
            "localhost:99999",
        ] {
            assert!(bare_hostname(invalid).is_none(), "{invalid:?}");
        }
    }

    #[test]
    fn certificate_fingerprints_accept_common_formats_and_prefixes() {
        let plain = "AB".repeat(32);
        let colons = ["ab"; 32].join(":");
        let hyphens = ["AB"; 32].join("-");
        let spaces = ["aB"; 32].join(" ");
        let openssl = format!("SHA256 Fingerprint={colons}");
        let prefixed = format!("sha-256: {plain}");

        for valid in [&plain, &colons, &hyphens, &spaces, &openssl, &prefixed] {
            assert_eq!(parse_fingerprint(valid), Some([0xab; 32]), "{valid:?}");
        }

        assert!(parse_fingerprint(&plain[..62]).is_none());
        assert!(parse_fingerprint(&plain.replace('B', "X")).is_none());
        let mixed = format!("ab:ab-{}", ["ab"; 30].join(":"));
        assert!(parse_fingerprint(&mixed).is_none());
    }

    #[test]
    fn certificate_pin_uses_the_canonical_hostname_and_fingerprint() {
        let pin = CertificatePin::parse("https://LOCALHOST:8006/", &"AB".repeat(32)).unwrap();
        assert_eq!(pin.hostname, "localhost");
        assert_eq!(pin.fingerprint_text(), "ab".repeat(32));
    }

    #[test]
    fn tls_requests_share_a_hostname_across_https_and_wss_ports() {
        assert_eq!(
            tls_hostname("https://LOCALHOST:8443/path").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            tls_hostname("wss://192.168.1.50:8006/api2/json/vncwebsocket").as_deref(),
            Some("192.168.1.50")
        );
        assert_eq!(tls_hostname("https://[::1]:8443/").as_deref(), Some("::1"));
        assert!(tls_hostname("http://localhost:8443").is_none());
        assert!(tls_hostname("ws://localhost:8443").is_none());
    }
}
