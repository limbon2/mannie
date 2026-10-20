use std::fs;

use super::can_change;

#[test]
fn a_file_of_the_account_itself_can_be_changed() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("worker.toml");
    fs::write(&file, "version = 1\n").unwrap();

    assert!(can_change(&file).unwrap());
}

#[cfg(unix)]
#[test]
fn a_read_only_file_in_a_read_only_folder_cannot_be_changed_unless_by_root() {
    use std::os::unix::fs::PermissionsExt;

    let folder = tempfile::tempdir().unwrap();
    let config = folder.path().join("etc");
    fs::create_dir(&config).unwrap();
    let file = config.join("worker.toml");
    fs::write(&file, "version = 1\n").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o555)).unwrap();

    let changeable = can_change(&file).unwrap();
    let is_root = rustix::process::getuid().is_root();

    fs::set_permissions(&config, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(changeable, is_root, "only root can change it");
}

#[cfg(windows)]
#[test]
fn a_read_only_file_in_a_folder_without_the_right_to_add_cannot_be_changed() {
    use std::process::Command;

    let folder = tempfile::tempdir().unwrap();
    let config = folder.path().join("etc");
    fs::create_dir(&config).unwrap();
    let file = config.join("worker.toml");
    fs::write(&file, "version = 1\n").unwrap();
    let icacls = std::path::Path::new(&std::env::var_os("SystemRoot").unwrap())
        .join("System32")
        .join("icacls.exe");
    let user = std::env::var("USERNAME").unwrap();
    // Replace permissions on the file as well as its folder. A file can
    // retain an explicit owner grant after the folder stops inheriting.
    let restricted = Command::new(&icacls)
        .arg(&config)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{user}:(OI)(CI)RX"))
        .arg("/T")
        .output()
        .unwrap();
    assert!(restricted.status.success(), "{restricted:?}");

    let changeable = can_change(&file).unwrap();

    let restored = Command::new(&icacls)
        .arg(&config)
        .args(["/reset", "/T", "/Q"])
        .output()
        .unwrap();
    assert!(restored.status.success(), "{restored:?}");
    assert!(!changeable);
}
