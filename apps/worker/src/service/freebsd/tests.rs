use super::rc_script;

fn script() -> String {
    rc_script(
        "/usr/local/libexec/mannie-worker/mannie-worker",
        "/usr/local/etc/mannie-worker/worker.toml",
        "/var/db/mannie-worker",
        "mannie",
    )
}

#[test]
fn the_script_runs_the_worker_under_daemon_as_the_account() {
    let script = script();

    let lines: Vec<&str> = script.lines().collect();
    for expected in [
        "#!/bin/sh",
        "# PROVIDE: mannie_worker",
        r#": "${mannie_worker_account:=mannie}""#,
        r#": "${mannie_worker_config:=/usr/local/etc/mannie-worker/worker.toml}""#,
        r#": "${mannie_worker_data_dir:=/var/db/mannie-worker}""#,
        r#"pidfile="/var/run/${name}.pid""#,
        r#"command_args="-f -r -R 10 -S -T ${name} -P ${pidfile} -u ${mannie_worker_account} /usr/local/libexec/mannie-worker/mannie-worker run --config ${mannie_worker_config}""#,
        "\tinstall -d -o \"${mannie_worker_account}\" -m 0700 \"${mannie_worker_data_dir}\"",
        r#"run_rc_command "$1""#,
    ] {
        assert!(
            lines.contains(&expected),
            "the script has no line {expected:?}:\n{script}"
        );
    }
}

#[test]
fn rc_subr_starts_daemon_as_root() {
    // rc.subr runs the command as `${name}_user` through su(1), and daemon(8)
    // needs root to start the worker as the account.
    let script = script();

    assert!(
        !script.contains("mannie_worker_user"),
        "the script names the account mannie_worker_user:\n{script}"
    );
}

#[test]
fn the_script_is_valid_shell() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("mannie_worker");
    std::fs::write(&path, script()).unwrap();

    let checked = std::process::Command::new("sh")
        .arg("-n")
        .arg(&path)
        .output()
        .unwrap();

    assert!(
        checked.status.success(),
        "sh -n failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}
