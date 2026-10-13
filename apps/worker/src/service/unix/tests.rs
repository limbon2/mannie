use std::path::Path;

use super::{existing_account, plain_path};

#[test]
fn accepts_plain_paths() {
    assert_eq!(
        plain_path(Path::new("/var/lib/mannie-worker"), "data folder").unwrap(),
        "/var/lib/mannie-worker"
    );
    assert_eq!(
        plain_path(Path::new("/opt/mannie/bin/mannie-worker"), "executable").unwrap(),
        "/opt/mannie/bin/mannie-worker"
    );
}

#[test]
fn refuses_paths_that_need_quoting() {
    for path in [
        "/var/lib/mannie worker",
        "/var/lib/$HOME",
        "/var/lib/a;b",
        "/var/lib/%i",
        "/var/lib/\"x\"",
        "/var/lib/a\nb",
        "",
    ] {
        let error = plain_path(Path::new(path), "data folder").unwrap_err();

        assert!(
            error.to_string().contains("cannot hold"),
            "{path:?}: {error}"
        );
    }
}

#[test]
fn refuses_account_names_that_are_not_names() {
    for user in ["", "-u", "a b", "root;id", "x/y", "$(id)"] {
        let error = existing_account(user).unwrap_err();

        assert!(
            error.to_string().contains("is not an account name"),
            "{user:?}: {error}"
        );
    }
}

#[test]
fn finds_an_existing_account() {
    existing_account("root").unwrap();
}

#[test]
fn reports_a_missing_account() {
    let error = existing_account("mannie-no-such-account").unwrap_err();

    assert!(error.to_string().contains("does not exist"), "{error}");
}
