use super::rc_script;

#[test]
fn the_script_runs_the_worker_under_daemon_as_the_account() {
    let script = rc_script(
        "/usr/local/bin/mannie-worker",
        "/var/db/mannie-worker",
        "mannie",
    );

    let lines: Vec<&str> = script.lines().collect();
    for expected in [
        "#!/bin/sh",
        "# PROVIDE: mannie_worker",
        r#": "${mannie_worker_user:=mannie}""#,
        r#": "${mannie_worker_data_dir:=/var/db/mannie-worker}""#,
        r#"command_args="-r -R 10 -S -T ${name} -P ${pidfile} -u ${mannie_worker_user} /usr/local/bin/mannie-worker run --data-dir ${mannie_worker_data_dir}""#,
        "\tinstall -d -o \"${mannie_worker_user}\" -m 0700 \"${mannie_worker_data_dir}\"",
        r#"run_rc_command "$1""#,
    ] {
        assert!(
            lines.contains(&expected),
            "the script has no line {expected:?}:\n{script}"
        );
    }
}

#[test]
fn the_script_is_valid_shell() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("mannie_worker");
    std::fs::write(
        &path,
        rc_script(
            "/usr/local/bin/mannie-worker",
            "/var/db/mannie-worker",
            "mannie",
        ),
    )
    .unwrap();

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
