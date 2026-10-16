use super::unit_file;

fn unit() -> String {
    unit_file(
        "/usr/local/libexec/mannie-worker/mannie-worker",
        "/etc/mannie-worker/worker.toml",
        "/var/lib/mannie-worker",
        "mannie",
    )
}

#[test]
fn the_unit_runs_the_worker_as_the_account_with_its_configuration() {
    let unit = unit();

    let lines: Vec<&str> = unit.lines().collect();
    for expected in [
        "User=mannie",
        "UMask=0077",
        "LimitCORE=0",
        "ExecStartPre=+/usr/bin/install -d -o mannie -m 0700 /var/lib/mannie-worker",
        "ExecStart=/usr/local/libexec/mannie-worker/mannie-worker run --config /etc/mannie-worker/worker.toml",
        "Restart=on-failure",
        "WantedBy=multi-user.target",
    ] {
        assert!(
            lines.contains(&expected),
            "the unit has no line {expected:?}:\n{unit}"
        );
    }
    assert!(
        !unit.contains("--allow-writable-config"),
        "a service never runs with a writable configuration"
    );
}

#[test]
fn the_unit_has_the_three_sections_in_order() {
    let unit = unit();

    let sections: Vec<&str> = unit.lines().filter(|line| line.starts_with('[')).collect();
    assert_eq!(sections, ["[Unit]", "[Service]", "[Install]"]);
}
