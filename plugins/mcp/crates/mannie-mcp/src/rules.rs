//! The rules of what the owner registers (plugins/mcp/docs/README.md,
//! registering a server). The settings check them as the owner types, and
//! the plugin checks them again in Rust, since the webview is not trusted
//! (ADR-0043, decision 6).

use mannie_plugin_sdk::contracts::{
    Url, clean_label, credential_transport_allowed, looks_like_secret,
};

/// The most characters of a server's name.
pub const MAX_NAME_CHARS: usize = 64;

/// The most characters of a server's program.
pub const MAX_PROGRAM_CHARS: usize = 1024;

/// The most arguments of a server's program.
pub const MAX_ARGUMENTS: usize = 64;

/// The most bytes of one argument.
pub const MAX_ARGUMENT_BYTES: usize = 4096;

/// The most characters of a server's working folder.
pub const MAX_FOLDER_CHARS: usize = 1024;

/// The most bytes of a server's URL.
pub const MAX_URL_BYTES: usize = 2048;

/// The most variables of a local server, plain and secret together.
pub const MAX_VARIABLES: usize = 32;

/// The most headers of a remote server, plain and secret together.
pub const MAX_HEADERS: usize = 32;

/// The most characters of a variable's or a header's name.
pub const MAX_NAME_OF_VALUE_CHARS: usize = 128;

/// The most bytes of a variable's or a header's value, plain or secret.
pub const MAX_VALUE_BYTES: usize = 8 * 1024;

/// The shortest timeout of a call of a server's tool, in seconds.
pub const MIN_TIMEOUT_SECONDS: u32 = 10;

/// The longest timeout of a call of a server's tool, in seconds.
pub const MAX_TIMEOUT_SECONDS: u32 = 3600;

/// The timeout of a new server's calls, in seconds.
pub const DEFAULT_TIMEOUT_SECONDS: u32 = 300;

/// The most characters of a client ID of OAuth that the owner registered.
pub const MAX_CLIENT_ID_CHARS: usize = 256;

/// The most bytes of a client secret of OAuth.
pub const MAX_CLIENT_SECRET_BYTES: usize = 1024;

/// The names of the headers that the transport sets itself, which a server's
/// own headers may not replace, in lower case. A name that starts with
/// `mcp-param-` belongs to the transport too.
const TRANSPORT_HEADERS: [&str; 12] = [
    "accept",
    "connection",
    "content-length",
    "content-type",
    "host",
    "last-event-id",
    "mcp-method",
    "mcp-name",
    "mcp-protocol-version",
    "mcp-session-id",
    "transfer-encoding",
    "upgrade",
];

/// The headers that are always secret, in lower case.
const SECRET_HEADERS: [&str; 4] = [
    "authorization",
    "cookie",
    "proxy-authorization",
    "x-api-key",
];

/// Whether a text is a server's name: 1 to [`MAX_NAME_CHARS`] characters as
/// the core shows a label, without control or invisible characters and
/// without white space at its ends or twice in a row, since approvals, the
/// journal and the system prompt show it.
pub fn is_server_name(name: &str) -> bool {
    name.chars().count() <= MAX_NAME_CHARS
        && clean_label(name, MAX_NAME_CHARS).as_deref() == Some(name)
}

/// Whether a text is a server's program: an absolute path or a name that the
/// core looks up, without control characters.
pub fn is_program(program: &str) -> bool {
    let count = program.chars().count();
    (1..=MAX_PROGRAM_CHARS).contains(&count)
        && program.trim() == program
        && !program.chars().any(char::is_control)
}

/// Whether arguments fit: at most [`MAX_ARGUMENTS`], each at most
/// [`MAX_ARGUMENT_BYTES`] and without NUL. Arguments may be empty and may
/// hold spaces: the program receives each as it is.
pub fn are_arguments(arguments: &[String]) -> bool {
    arguments.len() <= MAX_ARGUMENTS
        && arguments
            .iter()
            .all(|argument| argument.len() <= MAX_ARGUMENT_BYTES && !argument.contains('\0'))
}

/// Whether a working folder is an absolute path within its limit, without
/// control characters.
pub fn is_folder(folder: &str) -> bool {
    let count = folder.chars().count();
    (1..=MAX_FOLDER_CHARS).contains(&count)
        && std::path::Path::new(folder).is_absolute()
        && !folder.chars().any(char::is_control)
}

/// Whether a name is a variable's: a letter or an underscore, then letters,
/// digits and underscores.
pub fn is_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    name.len() <= MAX_NAME_OF_VALUE_CHARS
        && characters
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Whether a variable must be secret: its name looks like a secret's, such
/// as `GITHUB_TOKEN`.
pub fn variable_is_secret(name: &str) -> bool {
    looks_like_secret(name)
}

/// Whether a name is a header's: the characters of an HTTP token, and none
/// that the transport sets itself.
pub fn is_header_name(name: &str) -> bool {
    let is_token = !name.is_empty()
        && name.len() <= MAX_NAME_OF_VALUE_CHARS
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte));
    let lower = name.to_ascii_lowercase();
    let reserved = TRANSPORT_HEADERS.contains(&lower.as_str()) || lower.starts_with("mcp-param-");
    is_token && !reserved
}

/// Whether a header must be secret: one that carries credentials, or one
/// whose name looks like a secret's.
pub fn header_is_secret(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_HEADERS.contains(&lower.as_str()) || looks_like_secret(name)
}

/// Whether a value of a variable fits: at most [`MAX_VALUE_BYTES`], without
/// NUL.
pub fn is_variable_value(value: &str) -> bool {
    value.len() <= MAX_VALUE_BYTES && !value.contains('\0')
}

/// Whether a value of a header fits: at most [`MAX_VALUE_BYTES`] of visible
/// ASCII, spaces and tabs, as HTTP takes it.
pub fn is_header_value(value: &str) -> bool {
    value.len() <= MAX_VALUE_BYTES
        && value
            .bytes()
            .all(|byte| byte == b'\t' || (b' '..=b'~').contains(&byte))
}

/// Why a server's URL is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlRefusal {
    /// It is not an `http` or `https` URL with a host, within its limit.
    Invalid,
    /// It holds user information, a fragment or a query parameter whose
    /// name looks like a secret's, which would put a credential where
    /// approvals, the journal and grants show it.
    Credential,
    /// It is plain `http` to an address beyond this machine and the private
    /// networks, where a credential or what the tools say could be read on
    /// the way.
    Insecure,
}

/// The URL of a remote server, checked: `https`, or plain `http` only where
/// a provider's key may go (ADR-0043, decision 8), with no credential in it.
pub fn server_url(text: &str) -> Result<Url, UrlRefusal> {
    if text.len() > MAX_URL_BYTES || text.trim() != text {
        return Err(UrlRefusal::Invalid);
    }
    let url = Url::parse(text).map_err(|_error| UrlRefusal::Invalid)?;
    let is_http = matches!(url.scheme(), "http" | "https");
    if !is_http || url.host().is_none() {
        return Err(UrlRefusal::Invalid);
    }
    let has_user = !url.username().is_empty() || url.password().is_some();
    let secret_in_query = url
        .query_pairs()
        .any(|(name, _value)| looks_like_secret(&camel_case_apart(&name)));
    if has_user || url.fragment().is_some() || secret_in_query {
        return Err(UrlRefusal::Credential);
    }
    if !credential_transport_allowed(&url) {
        return Err(UrlRefusal::Insecure);
    }
    Ok(url)
}

/// A name with its camel-case words apart, such as `access_Token` for
/// `accessToken`, so that a secret's word inside it stands alone.
fn camel_case_apart(name: &str) -> String {
    let mut apart = String::with_capacity(name.len() + 4);
    let mut after_lower = false;
    for character in name.chars() {
        if after_lower && character.is_ascii_uppercase() {
            apart.push('_');
        }
        after_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
        apart.push(character);
    }
    apart
}

/// Whether a timeout is one that a server may have.
pub fn is_timeout(seconds: u32) -> bool {
    (MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&seconds)
}

/// Whether a client ID of OAuth fits: visible ASCII within its limit.
pub fn is_client_id(client_id: &str) -> bool {
    !client_id.is_empty()
        && client_id.len() <= MAX_CLIENT_ID_CHARS
        && client_id.bytes().all(|byte| (b'!'..=b'~').contains(&byte))
}

/// Whether a client secret of OAuth fits: visible ASCII within its limit.
pub fn is_client_secret(client_secret: &str) -> bool {
    !client_secret.is_empty()
        && client_secret.len() <= MAX_CLIENT_SECRET_BYTES
        && client_secret
            .bytes()
            .all(|byte| (b'!'..=b'~').contains(&byte))
}

/// Whether an argument looks like a token, such as one that a server's
/// documentation passes on its command line, which the settings warn about:
/// an argument leaves the server's process for the journal and approvals
/// (ADR-0043, decision 7).
pub fn looks_like_token(argument: &str) -> bool {
    const KNOWN_STARTS: [&str; 8] = [
        "ghp_",
        "gho_",
        "github_pat_",
        "glpat-",
        "sk-",
        "xoxb-",
        "xoxp-",
        "AKIA",
    ];
    if KNOWN_STARTS.iter().any(|start| argument.starts_with(start)) {
        return true;
    }
    if let Some((name, _value)) = argument.split_once('=')
        && looks_like_secret(&camel_case_apart(name.trim_start_matches('-')))
    {
        return true;
    }
    let value = argument
        .split_once('=')
        .map_or(argument, |(_name, value)| value);
    value.len() >= 32
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
        && value.chars().any(|character| character.is_ascii_digit())
        && value
            .chars()
            .any(|character| character.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::{
        UrlRefusal, header_is_secret, is_header_name, is_header_value, is_server_name,
        is_variable_name, looks_like_token, server_url, variable_is_secret,
    };

    #[test]
    fn a_server_name_is_short_and_without_control_characters() {
        assert!(is_server_name("GitHub"));
        assert!(is_server_name("Files (work)"));
        assert!(!is_server_name(""));
        assert!(!is_server_name(" GitHub"));
        assert!(!is_server_name("Git\u{7}Hub"));
        assert!(!is_server_name(&"a".repeat(65)));
    }

    #[test]
    fn variables_and_headers_have_names_of_their_kind_and_some_are_secret() {
        assert!(is_variable_name("GITHUB_TOKEN"));
        assert!(is_variable_name("_x1"));
        assert!(!is_variable_name("1X"));
        assert!(!is_variable_name("A-B"));
        assert!(variable_is_secret("GITHUB_PERSONAL_ACCESS_TOKEN"));
        assert!(!variable_is_secret("GITHUB_HOST"));
        assert!(is_header_name("X-Api-Key"));
        assert!(!is_header_name("Mcp-Session-Id"));
        assert!(!is_header_name("mcp-param-repo"));
        assert!(!is_header_name("Bad Header"));
        assert!(header_is_secret("Authorization"));
        assert!(header_is_secret("X-Auth-Token"));
        assert!(!header_is_secret("X-Team"));
        assert!(is_header_value("Bearer abc"));
        assert!(!is_header_value("line\nbreak"));
    }

    #[test]
    fn a_url_is_https_or_local_and_carries_no_credential() {
        server_url("https://api.githubcopilot.com/mcp/").unwrap();
        server_url("http://127.0.0.1:3000/mcp").unwrap();
        server_url("http://192.168.1.20:8080/mcp").unwrap();
        assert_eq!(
            server_url("http://mcp.example.com/mcp"),
            Err(UrlRefusal::Insecure)
        );
        assert_eq!(server_url("ftp://example.com/"), Err(UrlRefusal::Invalid));
        assert_eq!(server_url("not a url"), Err(UrlRefusal::Invalid));
        for credential in [
            "https://user:pass@mcp.example.com/mcp",
            "https://mcp.example.com/mcp?apiKey=abc",
            "https://mcp.example.com/mcp?access_token=abc",
            "https://mcp.example.com/mcp#token",
        ] {
            assert_eq!(
                server_url(credential),
                Err(UrlRefusal::Credential),
                "{credential}"
            );
        }
    }

    #[test]
    fn a_token_in_an_argument_is_found_for_a_warning() {
        assert!(looks_like_token("ghp_16C7e42F292c6912E7710c838347Ae178B4a"));
        assert!(looks_like_token("--api-key=abc"));
        assert!(looks_like_token("--token=abc"));
        assert!(looks_like_token("0123456789abcdefghijklmnopqrstuvwxyz"));
        assert!(!looks_like_token("-y"));
        assert!(!looks_like_token("@modelcontextprotocol/server-filesystem"));
        assert!(!looks_like_token("D:\\Projects\\app"));
    }
}
