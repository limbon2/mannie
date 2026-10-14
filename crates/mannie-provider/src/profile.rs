//! Checks of a profile that need a URL parser: the base URL, and where a
//! credential may travel (docs/features/chat/providers.md, credentials).

use std::net::{Ipv4Addr, Ipv6Addr};

use mannie_types::provider::ProviderProfileError;
use reqwest::Url;
use url::Host;

/// The longest base URL Mannie accepts, in bytes.
pub const MAX_BASE_URL_BYTES: usize = 2_048;

/// Checks a base URL: an absolute `http` or `https` URL without user
/// information, query or fragment.
pub fn check_base_url(text: &str) -> Result<Url, ProviderProfileError> {
    let text = text.trim();
    if text.len() > MAX_BASE_URL_BYTES {
        return Err(ProviderProfileError::InvalidBaseUrl);
    }
    let url = Url::parse(text).map_err(|_invalid| ProviderProfileError::InvalidBaseUrl)?;
    let is_http = matches!(url.scheme(), "http" | "https");
    let has_user = !url.username().is_empty() || url.password().is_some();
    let has_extras = url.query().is_some() || url.fragment().is_some();
    let has_host = url.host().is_some();
    if !is_http || has_user || has_extras || !has_host {
        return Err(ProviderProfileError::InvalidBaseUrl);
    }
    Ok(url)
}

/// Whether a credential may travel to `url`: always over `https`, and over
/// plain `http` only to this machine or to a private network address written
/// as an IP address. A host name could resolve anywhere, so only `localhost`
/// counts as local among names.
pub fn credential_transport_allowed(url: &Url) -> bool {
    if url.scheme() == "https" {
        return true;
    }
    match url.host() {
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => is_local_v4(address),
        Some(Host::Ipv6(address)) => is_local_v6(address),
        None => false,
    }
}

fn is_local_v4(address: Ipv4Addr) -> bool {
    let [first, second, ..] = address.octets();
    // 100.64.0.0/10 is the shared address space of carrier-grade NAT, which
    // overlay networks such as Tailscale use for their members.
    let is_shared = first == 100 && (64..128).contains(&second);
    address.is_loopback() || address.is_private() || address.is_link_local() || is_shared
}

fn is_local_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_local_v4(mapped);
    }
    let first = address.segments()[0];
    let is_unique_local = first & 0xfe00 == 0xfc00;
    let is_link_local = first & 0xffc0 == 0xfe80;
    address.is_loopback() || is_unique_local || is_link_local
}

#[cfg(test)]
mod tests {
    use mannie_types::provider::ProviderProfileError;

    use super::{check_base_url, credential_transport_allowed};

    #[test]
    fn base_urls_must_be_plain_http_or_https() {
        for valid in [
            "https://api.openai.com/v1",
            "http://127.0.0.1:8000/v1/",
            " http://gpu-box.local:11434/v1 ",
            "https://openrouter.ai/api/v1",
        ] {
            assert!(check_base_url(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "api.openai.com/v1",
            "ftp://example.com/v1",
            "https://user:secret@example.com/v1",
            "https://example.com/v1?key=secret",
            "https://example.com/v1#part",
            "file:///C:/models",
            "",
        ] {
            assert_eq!(
                check_base_url(invalid),
                Err(ProviderProfileError::InvalidBaseUrl),
                "{invalid}"
            );
        }
        let long = format!("https://example.com/{}", "a".repeat(2_100));
        assert_eq!(
            check_base_url(&long),
            Err(ProviderProfileError::InvalidBaseUrl)
        );
    }

    #[test]
    fn credentials_travel_over_http_only_to_local_addresses() {
        let allowed = [
            "https://api.openai.com/v1",
            "http://localhost:8000/v1",
            "http://127.0.0.1:8000/v1",
            "http://192.168.1.20:8000/v1",
            "http://10.0.0.5/v1",
            "http://172.16.3.4/v1",
            "http://100.101.102.103/v1",
            "http://[::1]:8000/v1",
            "http://[fd12:3456::1]/v1",
        ];
        let refused = [
            "http://gpu-box.local:8000/v1",
            "http://203.0.113.10/v1",
            "http://8.8.8.8/v1",
            "http://[2001:db8::1]/v1",
            "http://example.com/v1",
        ];
        for text in allowed {
            let url = check_base_url(text).unwrap();
            assert!(credential_transport_allowed(&url), "{text}");
        }
        for text in refused {
            let url = check_base_url(text).unwrap();
            assert!(!credential_transport_allowed(&url), "{text}");
        }
    }
}
