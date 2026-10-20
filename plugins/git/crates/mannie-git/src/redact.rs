//! Credentials in URLs (T-47; plugins/git/docs/approvals.md): a remote's URL
//! may carry a user and a password or a token, such as
//! `https://user:token@example.com/repo.git`, and git echoes URLs in its
//! messages. Every text that leaves the plugin, a result, a card's data, an
//! access, a log line or an error, goes through [`redact`] first, so no
//! credential reaches the model, the transcript, the journal or the logs.
//!
//! An HTTP URL loses its whole user part, since a token may stand alone as
//! the user. Other URLs, such as `ssh://git@example.com/`, keep the user and
//! lose a password. An scp-like address, such as `git@example.com:repo.git`,
//! names no password.

/// The schemes whose user part is dropped whole.
const HTTP_SCHEMES: [&str; 4] = ["http", "https", "ftp", "ftps"];

/// The text with the credentials of every URL in it removed.
pub(crate) fn redact(text: &str) -> String {
    let mut redacted = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(separator) = rest.find("://") {
        let scheme_start = rest[..separator]
            .char_indices()
            .rev()
            .take_while(|(_index, character)| {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
            })
            .last()
            .map_or(separator, |(index, _character)| index);
        let scheme = rest[scheme_start..separator].to_ascii_lowercase();
        let authority_start = separator + 3;
        let authority_end = rest[authority_start..]
            .find(|character: char| {
                matches!(character, '/' | '?' | '#' | '\'' | '"' | '<' | '>' | '`')
                    || character.is_whitespace()
            })
            .map_or(rest.len(), |offset| authority_start + offset);
        let authority = &rest[authority_start..authority_end];
        redacted.push_str(&rest[..authority_start]);
        redacted.push_str(&redact_authority(&scheme, authority));
        rest = &rest[authority_end..];
    }
    redacted.push_str(rest);
    redacted
}

/// An authority without its credentials: `host:port` for HTTP, and
/// `user@host:port` for other schemes.
fn redact_authority(scheme: &str, authority: &str) -> String {
    let Some((user_information, host)) = authority.rsplit_once('@') else {
        return authority.to_owned();
    };
    if HTTP_SCHEMES.contains(&scheme) {
        return host.to_owned();
    }
    let user = user_information
        .split_once(':')
        .map_or(user_information, |(user, _password)| user);
    format!("{user}@{host}")
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn http_urls_lose_their_user_and_password() {
        assert_eq!(
            redact("https://user:ghp_secret@github.com/owner/repo.git"),
            "https://github.com/owner/repo.git"
        );
        assert_eq!(
            redact("fatal: unable to access 'https://ghp_token@example.com/x/': 403"),
            "fatal: unable to access 'https://example.com/x/': 403"
        );
        assert_eq!(redact("HTTP://a:b@host"), "HTTP://host");
    }

    #[test]
    fn other_urls_keep_their_user_and_lose_a_password() {
        assert_eq!(
            redact("ssh://git:secret@example.com:22/repo.git"),
            "ssh://git@example.com:22/repo.git"
        );
        assert_eq!(
            redact("ssh://git@example.com/repo.git"),
            "ssh://git@example.com/repo.git"
        );
    }

    #[test]
    fn every_url_of_a_text_is_redacted_and_other_text_stays() {
        let text = "push https://a:b@one.example/r and pull https://c@two.example/r, git@three.example:r.git";

        assert_eq!(
            redact(text),
            "push https://one.example/r and pull https://two.example/r, git@three.example:r.git"
        );
        assert_eq!(redact("no URL here"), "no URL here");
        assert_eq!(redact("file:///srv/repo.git"), "file:///srv/repo.git");
        assert_eq!(redact("odd ://user:pw@host"), "odd ://user@host");
    }
}
