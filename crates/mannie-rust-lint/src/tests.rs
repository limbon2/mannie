use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{Rule, Status, check_file, run};

#[test]
fn reports_source_path_position_and_rule() {
    let path = Path::new("example.rs");
    let source = "fn check(count: usize, enabled: bool, a: bool, b: bool, c: bool, d: bool) -> bool {\n    count > 2 && enabled && a && b && c && d\n}";
    let mut output = Vec::new();
    let mut errors = Vec::new();

    let status = check_file(
        Rule::NamedConditions,
        path,
        source,
        &mut output,
        &mut errors,
    )
    .unwrap();

    assert_eq!(status, Status::Findings);
    assert!(
        String::from_utf8(output)
            .unwrap()
            .starts_with("example.rs:2:5: named-conditions:")
    );
    assert_eq!(errors.as_slice(), b"");
}

#[test]
fn reports_unparseable_source_as_incomplete() {
    let path = Path::new("example.rs");
    let mut output = Vec::new();
    let mut errors = Vec::new();

    let status = check_file(
        Rule::NamedConditions,
        path,
        "fn {",
        &mut output,
        &mut errors,
    )
    .unwrap();

    assert_eq!(status, Status::Incomplete);
    assert_eq!(output.as_slice(), b"");
    assert!(
        String::from_utf8(errors)
            .unwrap()
            .contains("could not parse Rust source")
    );
}

#[test]
fn reports_spacing_with_the_selected_rule() {
    let path = Path::new("example.rs");
    let source = "async fn save() {\n    prepare().await;\n    let value = collect();\n}";
    let mut output = Vec::new();
    let mut errors = Vec::new();

    let status = check_file(
        Rule::StatementGroupSpacing,
        path,
        source,
        &mut output,
        &mut errors,
    )
    .unwrap();

    assert_eq!(status, Status::Findings);
    assert!(
        String::from_utf8(output)
            .unwrap()
            .starts_with("example.rs:3:5: statement-group-spacing:")
    );
    assert_eq!(errors.as_slice(), b"");
}

#[test]
fn mode_flag_is_followed_by_source_paths() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("src/statement_group_spacing/tests.rs");
    let arguments = [
        OsString::from("--statement-group-spacing"),
        path.into_os_string(),
    ];
    let mut output = Vec::new();
    let mut errors = Vec::new();

    let status = run(&arguments, &mut output, &mut errors).unwrap();

    assert_eq!(status, Status::Clean);
    assert_eq!(output.as_slice(), b"");
    assert_eq!(errors.as_slice(), b"");
}
