//! Where a credential may travel (docs/features/chat/providers.md,
//! credentials): a provider profile's key, and the secret header or the
//! OAuth token of a tool server that a plugin reaches (ADR-0043, decision 8).

use std::net::{Ipv4Addr, Ipv6Addr};

use url::Host;
pub use url::Url;

/// Whether a credential may travel to `url`: always over `https`, and over
/// plain `http` only to this machine or to a private network address written
/// as an IP address. A host name could resolve anywhere, so only `localhost`
/// counts as local among names.
pub fn credential_transport_allowed(url: &Url) -> bool {
    if url.scheme() == "https" {
        return true;
    }
    if url.scheme() != "http" {
        return false;
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
    use url::Url;

    use super::credential_transport_allowed;

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
            "ftp://127.0.0.1/v1",
            "ws://localhost:8000/mcp",
        ];
        for text in allowed {
            let url = Url::parse(text).unwrap();
            assert!(credential_transport_allowed(&url), "{text}");
        }
        for text in refused {
            let url = Url::parse(text).unwrap();
            assert!(!credential_transport_allowed(&url), "{text}");
        }
    }
}
