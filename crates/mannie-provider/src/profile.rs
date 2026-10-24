//! Checks of a profile that need a URL parser: the base URL, and where a
//! credential may travel (docs/features/chat/providers.md, credentials).

use mannie_types::provider::ProviderProfileError;
use reqwest::Url;

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

// Where a credential may travel belongs to `mannie-types`, so that a
// plugin's tool server follows the same rule (ADR-0043, decision 8).
pub use mannie_types::transport::credential_transport_allowed;

#[cfg(test)]
mod tests {
    use mannie_types::provider::ProviderProfileError;

    use super::check_base_url;

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
}
