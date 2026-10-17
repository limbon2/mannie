//! The names of files that may hold secrets, which the "@" list marks with a
//! warning (docs/features/chat/mentions.md, decisions). Nothing is blocked:
//! the warning reminds the owner that a mention sends the file's text to the
//! model's provider.

/// File names that commonly hold keys, passwords or tokens.
const SECRET_NAMES: &[&str] = &[
    ".env",
    ".npmrc",
    ".pypirc",
    ".netrc",
    "_netrc",
    ".pgpass",
    ".git-credentials",
    ".htpasswd",
    "credentials",
    "credentials.json",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
];

/// Endings of names of keys, certificates and key stores.
const SECRET_ENDINGS: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"];

/// Endings of `.env.*` files that hold examples instead of secrets.
const EXAMPLE_ENDINGS: &[&str] = &[".example", ".sample", ".template"];

/// Folders whose files all may hold secrets.
const SECRET_FOLDERS: &[&str] = &[".ssh", ".aws", ".gnupg"];

/// Whether a file may hold secrets, by its path below the workspace with `/`
/// between names. Names compare without regard to case.
pub fn may_hold_secrets(path: &str) -> bool {
    let lower = path.to_lowercase();
    let mut names: Vec<&str> = lower.split('/').filter(|name| !name.is_empty()).collect();
    let Some(name) = names.pop() else {
        return false;
    };
    let in_secret_folder = names.iter().any(|folder| SECRET_FOLDERS.contains(folder));
    let is_env =
        name.starts_with(".env.") && !EXAMPLE_ENDINGS.iter().any(|ending| name.ends_with(ending));
    in_secret_folder
        || is_env
        || SECRET_NAMES.contains(&name)
        || name.starts_with("secrets.")
        || SECRET_ENDINGS.iter().any(|ending| name.ends_with(ending))
}

#[cfg(test)]
mod tests {
    use super::may_hold_secrets;

    #[test]
    fn env_files_may_hold_secrets_but_their_examples_do_not() {
        for path in [".env", "app/.env.local", ".ENV.production"] {
            assert!(may_hold_secrets(path), "{path}");
        }
        for path in [
            ".env.example",
            "app/.env.sample",
            ".env.template",
            ".envrc",
            "env.md",
        ] {
            assert!(!may_hold_secrets(path), "{path}");
        }
    }

    #[test]
    fn keys_and_credential_files_may_hold_secrets() {
        for path in [
            "certs/server.pem",
            "tls.KEY",
            "store.p12",
            "id_ed25519",
            "deploy/id_rsa",
            ".npmrc",
            "_netrc",
            "config/credentials.json",
            "secrets.yaml",
            "android/release.keystore",
        ] {
            assert!(may_hold_secrets(path), "{path}");
        }
        for path in [
            "id_ed25519.pub",
            "keys.md",
            "src/main.rs",
            "credentials.md",
            "README",
        ] {
            assert!(!may_hold_secrets(path), "{path}");
        }
    }

    #[test]
    fn every_file_in_a_secret_folder_may_hold_secrets() {
        assert!(may_hold_secrets("home/.ssh/config"));
        assert!(may_hold_secrets(".aws/config"));
        assert!(may_hold_secrets(".gnupg/pubring.kbx"));
        assert!(!may_hold_secrets("docs/ssh/config.md"));
        assert!(!may_hold_secrets(""));
    }
}
