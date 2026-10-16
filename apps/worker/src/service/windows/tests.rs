use std::ffi::OsString;
use std::path::Path;

use windows_service::service::{ServiceControlAccept, ServiceExitCode, ServiceState};

use super::{launch_arguments, service_status};

#[test]
fn the_service_control_manager_starts_the_worker_with_its_configuration() {
    let arguments = launch_arguments(Path::new(r"C:\ProgramData\Mannie\worker\worker.toml"));

    let expected: Vec<OsString> = [
        "service",
        "run",
        "--config",
        r"C:\ProgramData\Mannie\worker\worker.toml",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    assert_eq!(arguments, expected);
    assert!(
        !arguments
            .iter()
            .any(|argument| argument == "--allow-writable-config"),
        "a service never runs with a writable configuration"
    );
}

#[test]
fn a_running_service_accepts_stop_and_shutdown() {
    let status = service_status(ServiceState::Running, ServiceExitCode::NO_ERROR);

    assert_eq!(
        status.controls_accepted,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
    );
    assert!(status.wait_hint.is_zero());
}

#[test]
fn a_starting_service_accepts_nothing_and_asks_for_time() {
    let status = service_status(ServiceState::StartPending, ServiceExitCode::NO_ERROR);

    assert!(status.controls_accepted.is_empty());
    assert!(!status.wait_hint.is_zero());
}
