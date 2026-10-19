//! Links from model output (docs/features/chat/conversation-view.md, Markdown
//! rendering): the core checks a URL before the owner sees it in the
//! confirmation, and again before it opens in the system browser. The webview
//! never follows a link itself (ADR-0004).

use mannie_types::links::{CheckedLink, LinkError, MAX_URL_BYTES};
use tauri::Url;

use crate::state::Core;

/// Checks a link and returns it in the form the owner confirms: only `http`
/// and `https`, with a host, without user information, and with an
/// internationalized host name in its ASCII form, so that a look-alike host
/// shows as what it is.
pub(crate) fn check(url: &str) -> Result<CheckedLink, LinkError> {
    if url.len() > MAX_URL_BYTES {
        return Err(LinkError::TooLong {
            max_bytes: u32::try_from(MAX_URL_BYTES).unwrap_or(u32::MAX),
        });
    }
    // Browsers drop control characters and white space from URLs, so the
    // address that opens could differ from the one the owner read.
    if url
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(LinkError::InvalidUrl);
    }
    let parsed = Url::parse(url).map_err(|_invalid| LinkError::InvalidUrl)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(LinkError::UnsupportedScheme {
            scheme: parsed.scheme().to_owned(),
        });
    }
    // A user name before the host, as in https://bank.example@evil.example,
    // makes a link look as if it pointed to another host.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(LinkError::InvalidUrl);
    }
    let host = parsed
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or(LinkError::InvalidUrl)?
        .to_owned();
    Ok(CheckedLink {
        url: parsed.into(),
        host,
    })
}

/// Opens a link in the system browser after checking it again.
#[cfg(not(feature = "validation"))]
pub(crate) fn open(_core: &Core, url: &str) -> Result<(), LinkError> {
    let checked = check(url)?;
    tauri_plugin_opener::open_url(&checked.url, None::<&str>).map_err(|error| {
        tracing::warn!(%error, "the system browser could not be opened");
        LinkError::OpenFailed
    })?;
    tracing::info!(host = %checked.host, "opened a link in the system browser");
    Ok(())
}

/// Validation builds check the link and record it instead of starting a
/// browser, so that the end-to-end tests can see what would have opened.
#[cfg(feature = "validation")]
pub(crate) fn open(core: &Core, url: &str) -> Result<(), LinkError> {
    let checked = check(url)?;
    tracing::info!(host = %checked.host, "recorded a link instead of opening it (validation build)");
    core.record_opened_link(checked.url);
    Ok(())
}

#[cfg(test)]
mod tests {
    use mannie_types::links::{CheckedLink, LinkError};

    use super::check;

    #[test]
    fn web_links_are_normalized() {
        assert_eq!(
            check("https://Example.COM/path?q=1#top"),
            Ok(CheckedLink {
                url: "https://example.com/path?q=1#top".to_owned(),
                host: "example.com".to_owned(),
            })
        );
        assert_eq!(
            check("http://bücher.example/"),
            Ok(CheckedLink {
                url: "http://xn--bcher-kva.example/".to_owned(),
                host: "xn--bcher-kva.example".to_owned(),
            })
        );
        assert_eq!(
            check("https://example.com/\u{202e}gpj.exe").map(|link| link.url),
            Ok("https://example.com/%E2%80%AEgpj.exe".to_owned()),
            "a direction override shows as its escape, so it cannot reorder what the owner reads"
        );
    }

    #[test]
    fn other_schemes_and_disguised_links_are_refused() {
        for (url, expected) in [
            (
                "javascript:alert(1)",
                LinkError::UnsupportedScheme {
                    scheme: "javascript".to_owned(),
                },
            ),
            (
                "file:///C:/Windows/System32/calc.exe",
                LinkError::UnsupportedScheme {
                    scheme: "file".to_owned(),
                },
            ),
            (
                "data:text/html,<script>alert(1)</script>",
                LinkError::UnsupportedScheme {
                    scheme: "data".to_owned(),
                },
            ),
            (
                "mailto:owner@example.com",
                LinkError::UnsupportedScheme {
                    scheme: "mailto".to_owned(),
                },
            ),
            ("https://bank.example@evil.example/", LinkError::InvalidUrl),
            ("https://user:secret@example.com/", LinkError::InvalidUrl),
            ("https://example.com/a b", LinkError::InvalidUrl),
            ("https://example.com/\n", LinkError::InvalidUrl),
            ("not a url", LinkError::InvalidUrl),
            ("/relative/path", LinkError::InvalidUrl),
        ] {
            assert_eq!(check(url), Err(expected), "{url}");
        }
    }

    #[test]
    fn long_links_are_refused() {
        let long = format!("https://example.com/{}", "a".repeat(9_000));

        assert_eq!(check(&long), Err(LinkError::TooLong { max_bytes: 8_192 }));
    }
}
