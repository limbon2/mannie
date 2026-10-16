//! The worker as a Windows service. It runs as the virtual account
//! `NT SERVICE\mannie-worker`, which Windows creates with the service and which
//! needs no password. Only that account, SYSTEM and the administrators can
//! open the data folder; the configuration's folder belongs to SYSTEM and the
//! administrators, and the service's account can only read it (security
//! architecture, local data).

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use tokio::sync::Notify;
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use super::{SERVICE_NAME, report};
use crate::cli::RunArguments;
use crate::config::Config;
use crate::paths::Folders;
use crate::worker::{Worker, WorkerOptions};

const DISPLAY_NAME: &str = "Mannie worker";
const DESCRIPTION: &str =
    "Runs Mannie's agent runtime on this computer for a paired Mannie desktop app.";

/// The service's virtual account. Windows resolves it once the service exists.
const ACCOUNT: &str = r"NT SERVICE\mannie-worker";

/// The well-known SIDs of SYSTEM and the Administrators group. icacls takes
/// them with a leading `*`, which avoids the localized names.
const SYSTEM_SID: &str = "*S-1-5-18";
const ADMINISTRATORS_SID: &str = "*S-1-5-32-544";

/// How long the commands wait for the service to start or stop.
const STATE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the service control manager waits between status reports while
/// the worker starts or stops.
const PENDING_WAIT_HINT: Duration = Duration::from_secs(30);

/// Win32 errors that the commands explain.
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
const ERROR_SERVICE_NOT_ACTIVE: i32 = 1062;
const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;
const ERROR_SERVICE_EXISTS: i32 = 1073;

pub(super) fn install(program: &Path, config_file: &Path, config: &Config) -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(explain)?;
    let info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: program.to_path_buf(),
        launch_arguments: launch_arguments(config_file),
        dependencies: Vec::new(),
        account_name: Some(ACCOUNT.into()),
        account_password: None,
    };
    let access = ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::START
        | ServiceAccess::QUERY_STATUS
        | ServiceAccess::DELETE;
    let service = manager.create_service(&info, access).map_err(explain)?;
    report(&format!(
        "Created the service {SERVICE_NAME}, which runs as {ACCOUNT}."
    ));
    let config_folder = config_file.parent().unwrap_or(config_file);
    let prepared = configure(&service)
        .and_then(|()| restrict_config_directory(config_folder))
        .and_then(|()| restrict_data_directory(&config.data_dir));
    if let Err(error) = prepared {
        // Leave nothing half installed.
        return match service.delete() {
            Ok(()) => Err(error.context("the service was removed again")),
            Err(delete_error) => Err(error.context(format!(
                "removing the incomplete service failed too ({delete_error}); run `mannie-worker service uninstall`"
            ))),
        };
    }
    report(&format!(
        "SYSTEM and the administrators own {}, and {ACCOUNT} can only read it.",
        config_folder.display()
    ));
    report(&format!(
        "Only {ACCOUNT}, SYSTEM and the administrators can open {}.",
        config.data_dir.display()
    ));
    service
        .start::<&OsStr>(&[])
        .map_err(explain)
        .context("the service is installed but did not start")?;
    wait_for_state(&service, ServiceState::Running).context(
        "the service is installed but did not start. The System log in Event Viewer and `mannie-worker status` show why",
    )?;
    report(&format!(
        "Started the service {SERVICE_NAME}. It starts with Windows."
    ));
    Ok(())
}

pub(super) fn uninstall() -> anyhow::Result<()> {
    let service = open(ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)?;
    if stop_service(&service)? {
        report(&format!("Stopped the service {SERVICE_NAME}."));
    }
    service.delete().map_err(explain)?;
    report(&format!("Removed the service {SERVICE_NAME}."));
    Ok(())
}

pub(super) fn is_installed() -> anyhow::Result<bool> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(explain)?;
    match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
        Ok(_service) => Ok(true),
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST) =>
        {
            Ok(false)
        }
        Err(error) => Err(explain(error)),
    }
}

pub(super) fn is_running() -> anyhow::Result<bool> {
    let service = open(ServiceAccess::QUERY_STATUS)?;
    let state = service.query_status().map_err(explain)?.current_state;
    Ok(state == ServiceState::Running)
}

pub(super) fn stop() -> anyhow::Result<()> {
    let service = open(ServiceAccess::QUERY_STATUS | ServiceAccess::STOP)?;
    stop_service(&service)?;
    Ok(())
}

pub(super) fn start() -> anyhow::Result<()> {
    let service = open(ServiceAccess::QUERY_STATUS | ServiceAccess::START)?;
    service.start::<&OsStr>(&[]).map_err(explain)?;
    wait_for_state(&service, ServiceState::Running)
}

/// Opens the installed service.
fn open(access: ServiceAccess) -> anyhow::Result<Service> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(explain)?;
    manager.open_service(SERVICE_NAME, access).map_err(explain)
}

/// Stops the service and waits until it stopped. Returns whether it ran.
fn stop_service(service: &Service) -> anyhow::Result<bool> {
    let state = service.query_status().map_err(explain)?.current_state;
    if state == ServiceState::Stopped {
        return Ok(false);
    }
    match service.stop() {
        Ok(_status) => {}
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_NOT_ACTIVE) => {}
        Err(error) => return Err(explain(error)).context("could not stop the service"),
    }
    wait_for_state(service, ServiceState::Stopped).context("the service did not stop")?;
    Ok(true)
}

/// The arguments the service control manager starts the worker with.
fn launch_arguments(config_file: &Path) -> Vec<OsString> {
    vec![
        "service".into(),
        "run".into(),
        "--config".into(),
        config_file.as_os_str().to_owned(),
    ]
}

/// Sets the description and restarts after a failure: twice, ten seconds
/// apart, then not again until a day without failures has passed.
fn configure(service: &Service) -> anyhow::Result<()> {
    service.set_description(DESCRIPTION).map_err(explain)?;
    let restart = ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(10),
    };
    let give_up = ServiceAction {
        action_type: ServiceActionType::None,
        delay: Duration::ZERO,
    };
    service
        .update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_hours(24)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![restart.clone(), restart, give_up]),
        })
        .map_err(explain)?;
    // A worker that exits with an error, such as a database from a newer
    // version, counts as a failure too.
    service
        .set_failure_actions_on_non_crash_failures(true)
        .map_err(explain)?;
    Ok(())
}

/// Gives the configuration's folder an access list of its own: full control
/// for SYSTEM and the administrators, and read access for the service's
/// account. Without it the folder would inherit the right of every user to
/// add files under `%ProgramData%`.
fn restrict_config_directory(directory: &Path) -> anyhow::Result<()> {
    set_access(directory, "RX")
}

/// Gives the data folder an access list of its own: full control for SYSTEM
/// and the administrators, and modify rights for the service account.
fn restrict_data_directory(directory: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(directory)
        .with_context(|| format!("could not create the data folder {}", directory.display()))?;
    set_access(directory, "M")
}

/// Replaces a folder's access list: the first icacls call drops explicit
/// entries in the whole folder; the second removes the inherited entries and
/// grants full control to SYSTEM and the administrators and `rights` to the
/// service's account, which the folder's contents then inherit.
fn set_access(directory: &Path, rights: &str) -> anyhow::Result<()> {
    let icacls = icacls_path()?;
    let directory = directory.as_os_str();
    run_icacls(
        &icacls,
        &[directory, "/reset".as_ref(), "/T".as_ref(), "/Q".as_ref()],
    )?;
    let service_grant = format!("{ACCOUNT}:(OI)(CI){rights}");
    let system_grant = format!("{SYSTEM_SID}:(OI)(CI)F");
    let administrators_grant = format!("{ADMINISTRATORS_SID}:(OI)(CI)F");
    run_icacls(
        &icacls,
        &[
            directory,
            "/inheritance:r".as_ref(),
            "/grant:r".as_ref(),
            system_grant.as_ref(),
            administrators_grant.as_ref(),
            service_grant.as_ref(),
            "/Q".as_ref(),
        ],
    )
}

/// icacls from the Windows folder, never from the search path.
fn icacls_path() -> anyhow::Result<PathBuf> {
    let windows =
        std::env::var_os("SystemRoot").context("the SystemRoot environment variable is missing")?;
    Ok(Path::new(&windows).join("System32").join("icacls.exe"))
}

fn run_icacls(icacls: &Path, arguments: &[&OsStr]) -> anyhow::Result<()> {
    let output = Command::new(icacls)
        .args(arguments)
        .output()
        .with_context(|| format!("could not run {}", icacls.display()))?;
    if output.status.success() {
        return Ok(());
    }
    // icacls writes its errors to standard output, in the system's language.
    let message = String::from_utf8_lossy(&output.stdout);
    let shown: Vec<_> = arguments
        .iter()
        .map(|argument| argument.to_string_lossy())
        .collect();
    bail!(
        "`icacls {}` failed ({}): {}",
        shown.join(" "),
        output.status,
        message.trim()
    )
}

/// Polls the service until it reaches `wanted`, or fails when it stops on the way.
fn wait_for_state(service: &Service, wanted: ServiceState) -> anyhow::Result<()> {
    let deadline = Instant::now() + STATE_TIMEOUT;
    loop {
        let status = service.query_status().map_err(explain)?;
        if status.current_state == wanted {
            return Ok(());
        }
        let stopped_instead = status.current_state == ServiceState::Stopped;
        if stopped_instead {
            bail!("the service stopped with exit code {:?}", status.exit_code);
        }
        if Instant::now() > deadline {
            bail!(
                "the service did not reach {wanted:?} within {} seconds",
                STATE_TIMEOUT.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// Turns the Win32 errors that the owner can act on into instructions.
fn explain(error: windows_service::Error) -> anyhow::Error {
    let code = match &error {
        windows_service::Error::Winapi(io_error) => io_error.raw_os_error(),
        _ => None,
    };
    match code {
        Some(ERROR_ACCESS_DENIED) => {
            anyhow!(error).context("Windows refused access. Run the command in a terminal opened as administrator")
        }
        Some(ERROR_SERVICE_EXISTS) => anyhow!(error)
            .context(format!("the service {SERVICE_NAME} is already installed. Run `mannie-worker service uninstall` first")),
        Some(ERROR_SERVICE_DOES_NOT_EXIST) => anyhow!(error).context(format!("the service {SERVICE_NAME} is not installed")),
        _ => anyhow!(error),
    }
}

/// The arguments the service control manager started the worker with, for
/// the service's main function.
static ARGUMENTS: OnceLock<RunArguments> = OnceLock::new();

windows_service::define_windows_service!(service_entry, service_main);

/// Hands the process to the service control manager, which calls
/// `service_main` on another thread and returns once the service stopped.
pub(super) fn run(arguments: &RunArguments) -> anyhow::Result<()> {
    ARGUMENTS.get_or_init(|| arguments.clone());
    service_dispatcher::start(SERVICE_NAME, service_entry).map_err(|error| match &error {
        windows_service::Error::Winapi(io_error) if io_error.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) => {
            anyhow!(error).context("Windows starts the service with this command. To run the worker in a terminal, use `mannie-worker run`")
        }
        _ => anyhow!(error),
    })
}

fn service_main(_arguments: Vec<OsString>) {
    let Some(arguments) = ARGUMENTS.get() else {
        return;
    };
    let stop = Arc::new(Notify::new());
    let stop_requested = Arc::clone(&stop);
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            stop_requested.notify_one();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    // Without a status handle the service cannot report anything; the service
    // control manager times the start out and records that in the System log.
    let Ok(status) = service_control_handler::register(SERVICE_NAME, handler) else {
        return;
    };
    let exit_code = match serve(status, arguments, &stop) {
        Ok(()) => ServiceExitCode::NO_ERROR,
        Err(error) => {
            tracing::error!(error = format!("{error:#}"), "the worker service failed");
            ServiceExitCode::ServiceSpecific(1)
        }
    };
    if let Err(error) = status.set_service_status(service_status(ServiceState::Stopped, exit_code))
    {
        tracing::error!(%error, "could not report that the service stopped");
    }
}

/// Starts the worker, reports it running, and stops it when the service
/// control manager asks.
fn serve(
    status: ServiceStatusHandle,
    arguments: &RunArguments,
    stop: &Notify,
) -> anyhow::Result<()> {
    status.set_service_status(service_status(
        ServiceState::StartPending,
        ServiceExitCode::NO_ERROR,
    ))?;
    let folders = Folders::of_this_system()?;
    let config = crate::run::configuration(arguments, &folders)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the async runtime")?;
    runtime.block_on(async {
        let options = WorkerOptions {
            config,
            folders,
            console_logging: false,
        };
        let worker = Worker::start(&options).await?;
        status.set_service_status(service_status(
            ServiceState::Running,
            ServiceExitCode::NO_ERROR,
        ))?;
        worker
            .run_until(async {
                stop.notified().await;
                if let Err(error) = status.set_service_status(service_status(
                    ServiceState::StopPending,
                    ServiceExitCode::NO_ERROR,
                )) {
                    tracing::warn!(%error, "could not report that the service stops");
                }
            })
            .await
    })
}

fn service_status(state: ServiceState, exit_code: ServiceExitCode) -> ServiceStatus {
    let is_running = state == ServiceState::Running;
    let is_pending = matches!(
        state,
        ServiceState::StartPending | ServiceState::StopPending
    );
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if is_running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code,
        checkpoint: 0,
        wait_hint: if is_pending {
            PENDING_WAIT_HINT
        } else {
            Duration::ZERO
        },
        process_id: None,
    }
}

#[cfg(test)]
mod tests;
