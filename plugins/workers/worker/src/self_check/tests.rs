use super::{RELEASE_KEY_SPKI, release_key, this_program};

#[test]
fn the_check_names_this_program_and_the_release_key() {
    let check = this_program();

    assert_eq!(check.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(check.os, std::env::consts::OS);
    assert_eq!(check.release_key_sha256.len(), 64);
    assert!(
        check
            .release_key_sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit()),
        "{}",
        check.release_key_sha256
    );
}

#[test]
fn the_raw_key_is_the_end_of_the_key_info() {
    assert_eq!(release_key().len(), 32);
    assert!(RELEASE_KEY_SPKI.ends_with(release_key()));
}

#[test]
fn the_json_keeps_its_shape() {
    let json = serde_json::to_value(this_program()).unwrap();

    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["arch", "os", "releaseKeySha256", "version"]);
}
