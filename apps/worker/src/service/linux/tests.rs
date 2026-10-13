use super::unit_file;

#[test]
fn the_unit_runs_the_worker_as_the_account_with_its_data_folder() {
    let unit = unit_file(
        "/usr/local/bin/mannie-worker",
        "/var/lib/mannie-worker",
        "mannie",
    );

    let lines: Vec<&str> = unit.lines().collect();
    for expected in [
        "User=mannie",
        "UMask=0077",
        "ExecStartPre=+/usr/bin/install -d -o mannie -m 0700 /var/lib/mannie-worker",
        "ExecStart=/usr/local/bin/mannie-worker run --data-dir /var/lib/mannie-worker",
        "Restart=on-failure",
        "WantedBy=multi-user.target",
    ] {
        assert!(
            lines.contains(&expected),
            "the unit has no line {expected:?}:\n{unit}"
        );
    }
}

#[test]
fn the_unit_has_the_three_sections_in_order() {
    let unit = unit_file(
        "/usr/local/bin/mannie-worker",
        "/var/lib/mannie-worker",
        "mannie",
    );

    let sections: Vec<&str> = unit.lines().filter(|line| line.starts_with('[')).collect();
    assert_eq!(sections, ["[Unit]", "[Service]", "[Install]"]);
}
