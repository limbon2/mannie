//! The Windows half of the process supervisor (ADR-0014, docs/features/
//! system-tools/commands.md, Windows).
//!
//! Each command runs in a Job Object of its own. The shell starts suspended
//! and without a window, joins the job before its first thread runs, and is
//! then resumed. The job ends every process in it when its last handle
//! closes, suppresses error dialogs, and lets no process break away, so
//! terminating it, or Mannie ending, ends every process the command started.
#![expect(
    unsafe_code,
    reason = "the process supervisor calls Win32 functions that windows-sys exposes as unsafe (ADR-0014)"
)]

use std::ffi::OsString;
use std::io;
use std::mem;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::PathBuf;
use std::process::Stdio;
use std::ptr;

use mannie_types::tools::ProcessRecord;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use windows_sys::Win32::Foundation::{
    FILETIME, HANDLE, INVALID_HANDLE_VALUE, MAX_PATH, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, GetProcessTimes, OpenProcess, OpenThread,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    ResumeThread, THREAD_SUSPEND_RESUME, TerminateProcess, WaitForSingleObject,
};

use super::CommandSpec;
use super::server::{ServerInput, ServerSpec};
use crate::programs::ProgramRun;

/// The script that Windows PowerShell runs. It reads the command's text from
/// standard input as UTF-8, sets UTF-8 for output, turns off progress output
/// and runs the text as one script block, so blocks and here-strings that
/// span lines work. A last line inside the block records whether the
/// command's last statement succeeded: the exit code is the command's own
/// `exit`, else the exit code of a program whose failure ended the command,
/// else 1 for a failed statement, else 0. The script has no double quotes,
/// so the command line carries it unchanged.
const BOOTSTRAP: &str = "$ProgressPreference='SilentlyContinue';\
$utf8=New-Object System.Text.UTF8Encoding $false;\
[Console]::OutputEncoding=$utf8;\
$OutputEncoding=$utf8;\
$reader=New-Object System.IO.StreamReader([Console]::OpenStandardInput(),$utf8);\
$text=$reader.ReadToEnd();\
$reader.Dispose();\
$global:LASTEXITCODE=0;\
$mannieLastStatus=$true;\
. ([ScriptBlock]::Create($text+[Environment]::NewLine+'$mannieLastStatus=$?'));\
if($mannieLastStatus){exit 0};\
if($global:LASTEXITCODE){exit $global:LASTEXITCODE};\
exit 1";

/// The system folder, such as `C:\Windows\System32`, from Windows itself
/// rather than from an environment variable.
pub(super) fn system_directory() -> io::Result<PathBuf> {
    let mut buffer = vec![0_u16; MAX_PATH as usize];
    loop {
        let capacity = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        // SAFETY: the pointer and the capacity describe `buffer`, which lives
        // until the call returns; the function writes at most `capacity`
        // UTF-16 units into it.
        let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), capacity) };
        let length = usize::try_from(length).unwrap_or(usize::MAX);
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        // When the buffer is too small, the result is the size it needs,
        // with the terminating NUL.
        if length < buffer.len() {
            buffer.truncate(length);
            return Ok(PathBuf::from(OsString::from_wide(&buffer)));
        }
        buffer.resize(length, 0);
    }
}

/// A running command: its shell and the job that holds its processes.
#[derive(Debug)]
pub(super) struct Contained {
    job: OwnedHandle,
    record: ProcessRecord,
}

impl Contained {
    /// Starts the shell in a new job and gives it the command's text.
    pub(super) async fn spawn(spec: &CommandSpec<'_>) -> io::Result<(Self, Child)> {
        let job = create_job()?;
        let mut command = Command::new(spec.shell);
        command
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .raw_arg(format!("\"{BOOTSTRAP}\""))
            .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
            .current_dir(spec.working_directory)
            .env_clear()
            .envs(spec.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let contained = Self::contain(job, &mut child)?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(spec.text.as_bytes()).await?;
            // Closing standard input ends the shell's read of the text.
            drop(stdin);
        }
        Ok((contained, child))
    }

    /// Starts an installed program in a new job, with its arguments exactly
    /// and no shell (ADR-0038). The standard library quotes each argument
    /// for the program's own parsing of its command line; the resolver only
    /// ever hands over `.exe` files, whose parsing follows those rules, and
    /// never `.bat` or `.cmd` files, which `cmd.exe` would read.
    pub(super) fn spawn_program(run: &ProgramRun) -> io::Result<(Self, Child)> {
        let job = create_job()?;
        let mut command = Command::new(&run.program);
        command
            .args(&run.arguments)
            .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
            .current_dir(&run.working_directory)
            .env_clear()
            .envs(run.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let contained = Self::contain(job, &mut child)?;
        Ok((contained, child))
    }

    /// Starts a local server's program in a new job, with its arguments
    /// exactly and its input open (ADR-0043, decision 5). The standard
    /// library starts a `.cmd` or `.bat` file through `cmd.exe`, quoting
    /// each argument for it, and refuses an argument that `cmd.exe` would
    /// read as code.
    pub(super) fn spawn_server(spec: &ServerSpec) -> io::Result<(Self, Child, ServerInput)> {
        let job = create_job()?;
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.arguments)
            .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
            .current_dir(&spec.working_directory)
            .env_clear()
            .envs(spec.environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let contained = Self::contain(job, &mut child)?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("the server has no standard input"))?;
        Ok((contained, child, Box::new(input)))
    }

    /// Puts a suspended process into its job, and only then lets it run.
    fn contain(job: OwnedHandle, child: &mut Child) -> io::Result<Self> {
        let Some(process) = child.raw_handle() else {
            return Err(io::Error::other("the process exited before it started"));
        };
        let pid = child.id().unwrap_or(0);
        let joined = assign(&job, process).and_then(|()| resume(pid));
        if let Err(error) = joined {
            // The process is still suspended: end it before it runs anything.
            terminate(&job);
            child.start_kill()?;
            return Err(error);
        }
        let record = ProcessRecord {
            pid,
            start_time: creation_time(process).unwrap_or_default(),
        };
        Ok(Self { job, record })
    }

    /// Gives a program its input, written while its output is read, so a
    /// program that writes before it reads cannot block on a full pipe.
    #[expect(
        clippy::unused_self,
        reason = "the Unix half keeps the input pipe on the run, and both halves share one signature"
    )]
    pub(super) fn send_input(&mut self, child: &mut Child, input: Vec<u8>) {
        let Some(mut stdin) = child.stdin.take() else {
            return;
        };
        tokio::spawn(async move {
            // A program that ends without reading all of it closes the pipe,
            // and the write fails; nothing waits for the rest.
            if let Err(error) = stdin.write_all(&input).await {
                tracing::debug!(%error, "a program did not read all of its input");
            }
            // Dropping the pipe closes the program's standard input.
        });
    }

    /// The shell's process, for the journal.
    pub(super) fn record(&self) -> ProcessRecord {
        self.record.clone()
    }

    /// Waits until the shell exits, and returns its exit code.
    pub(super) async fn finished(&mut self, child: &mut Child) -> Option<i32> {
        child.wait().await.ok().and_then(|status| status.code())
    }

    /// Ends every process of the command and waits for the shell. A command
    /// that ended on its own leaves processes behind only when it started
    /// some; terminating the job ends them too.
    pub(super) async fn end(&mut self, child: &mut Child, _graceful: bool) {
        terminate(&self.job);
        if let Err(error) = child.wait().await {
            tracing::warn!(%error, "the shell of a command was not waited for");
        }
    }
}

/// Creates a job that ends its processes when its last handle closes, shows
/// no error dialogs, and lets no process break away. Its handle is not
/// inherited.
pub(super) fn create_job() -> io::Result<OwnedHandle> {
    // SAFETY: null attributes and a null name are valid arguments; the result
    // is checked before it is used.
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a new, valid handle that nothing else owns.
    let job = unsafe { OwnedHandle::from_raw_handle(raw as RawHandle) };
    // SAFETY: the structure is plain data, for which all zeros is a valid
    // value.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
    let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
        .map_err(io::Error::other)?;
    // SAFETY: the job handle is valid, and the pointer and size describe
    // `limits`, which lives until the call returns.
    let set = unsafe {
        SetInformationJobObject(
            job.as_raw_handle() as HANDLE,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size,
        )
    };
    if set == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

/// Puts the suspended shell into the job.
pub(super) fn assign(job: &OwnedHandle, process: RawHandle) -> io::Result<()> {
    // SAFETY: both handles are valid: the job is owned here, and the process
    // handle belongs to the child, which outlives the call.
    let assigned =
        unsafe { AssignProcessToJobObject(job.as_raw_handle() as HANDLE, process as HANDLE) };
    if assigned == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Resumes the threads of a suspended process. A process created suspended
/// has one thread, which the standard library does not hand out, so the
/// thread is found in a snapshot of the system's threads.
fn resume(pid: u32) -> io::Result<()> {
    // SAFETY: the flags and the process ID are valid arguments; the result is
    // checked before it is used.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a new, valid handle that nothing else owns.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw as RawHandle) };
    let mut entry = THREADENTRY32 {
        dwSize: u32::try_from(size_of::<THREADENTRY32>()).map_err(io::Error::other)?,
        cntUsage: 0,
        th32ThreadID: 0,
        th32OwnerProcessID: 0,
        tpBasePri: 0,
        tpDeltaPri: 0,
        dwFlags: 0,
    };
    let mut resumed = 0_u32;
    // SAFETY: the snapshot handle is valid, and `entry` is a writable
    // structure whose size field is set, as the function requires.
    let mut found = unsafe { Thread32First(snapshot.as_raw_handle() as HANDLE, &raw mut entry) };
    while found != 0 {
        if entry.th32OwnerProcessID == pid {
            resume_thread(entry.th32ThreadID)?;
            resumed += 1;
        }
        // SAFETY: as for `Thread32First`.
        found = unsafe { Thread32Next(snapshot.as_raw_handle() as HANDLE, &raw mut entry) };
    }
    if resumed == 0 {
        return Err(io::Error::other("the shell's thread was not found"));
    }
    Ok(())
}

/// Resumes one thread.
fn resume_thread(thread_id: u32) -> io::Result<()> {
    // SAFETY: the access right and the thread ID are valid arguments; the
    // handle is not inherited, and the result is checked before it is used.
    let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a new, valid handle that nothing else owns.
    let thread = unsafe { OwnedHandle::from_raw_handle(raw as RawHandle) };
    // SAFETY: the thread handle is valid and has the right to resume.
    let previous = unsafe { ResumeThread(thread.as_raw_handle() as HANDLE) };
    if previous == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Ends every process in the job.
pub(super) fn terminate(job: &OwnedHandle) {
    // SAFETY: the job handle is valid for the call.
    let terminated = unsafe { TerminateJobObject(job.as_raw_handle() as HANDLE, 1) };
    if terminated == 0 {
        tracing::warn!(
            error = %io::Error::last_os_error(),
            "the job of a command was not terminated"
        );
    }
}

/// The time the process was created, in 100-nanosecond intervals since 1601,
/// which tells it apart from a later process with the same ID.
pub(super) fn creation_time(process: RawHandle) -> Option<String> {
    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    // SAFETY: the process handle is valid, and the four pointers point to
    // writable structures that live until the call returns.
    let got = unsafe {
        GetProcessTimes(
            process as HANDLE,
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    };
    (got != 0).then(|| {
        let time = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        time.to_string()
    })
}

/// A kill-on-close job for a process tree that Mannie did not start itself,
/// such as the browser process of a WebView2 environment (ADR-0064,
/// decision 4). The tree's processes join it after they started, so a
/// process that their root started earlier, or that left the job, is outside
/// it; the caller ends those through their root.
#[derive(Debug)]
pub struct ProcessTreeJob {
    job: OwnedHandle,
}

impl ProcessTreeJob {
    /// Creates an empty job that ends its processes when it is dropped.
    ///
    /// # Errors
    ///
    /// The system's error when the job cannot be created.
    pub fn new() -> io::Result<Self> {
        Ok(Self { job: create_job()? })
    }

    /// Puts a running process into the job. A process that is in it already
    /// succeeds.
    ///
    /// # Errors
    ///
    /// The system's error when the process cannot be opened or assigned.
    pub fn assign(&self, process_id: u32) -> io::Result<()> {
        let process = open_process(
            process_id,
            PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
        )?;
        self.assign_handle(&process)
    }

    /// Assigns a browser process to the job and retains its exact identity for
    /// later teardown. If assignment fails, terminate the process we opened.
    ///
    /// # Errors
    ///
    /// The system's error if the process cannot be opened or assigned.
    pub fn adopt(&self, process_id: u32) -> io::Result<ProcessHandle> {
        let process = ProcessHandle(open_process(
            process_id,
            PROCESS_SET_QUOTA
                | PROCESS_TERMINATE
                | PROCESS_QUERY_LIMITED_INFORMATION
                | PROCESS_SYNCHRONIZE,
        )?);
        if let Err(error) = self.assign_handle(&process.0) {
            process.terminate();
            return Err(error);
        }
        Ok(process)
    }

    fn assign_handle(&self, process: &OwnedHandle) -> io::Result<()> {
        let mut inside = 0;
        // SAFETY: both handles are valid for the call, and `inside` is a
        // writable BOOL that lives until it returns.
        let checked = unsafe {
            IsProcessInJob(
                process.as_raw_handle() as HANDLE,
                self.job.as_raw_handle() as HANDLE,
                &raw mut inside,
            )
        };
        if checked != 0 && inside != 0 {
            return Ok(());
        }
        assign(&self.job, process.as_raw_handle())
    }

    /// Ends every process in the job at once.
    ///
    /// # Errors
    ///
    /// The system's error when the job cannot be terminated.
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: the job handle is valid for as long as `self` lives.
        let ended = unsafe { TerminateJobObject(self.job.as_raw_handle() as HANDLE, 1) };
        if ended == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// The number of processes that are still in the job.
    ///
    /// # Errors
    ///
    /// The system's error when the job cannot be queried.
    pub fn active_processes(&self) -> io::Result<u32> {
        // SAFETY: the structure is plain data, for which all zeros is a valid
        // value.
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { mem::zeroed() };
        let size = u32::try_from(size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>())
            .map_err(io::Error::other)?;
        // SAFETY: the job handle is valid, and the pointer and size describe
        // `accounting`, which lives until the call returns.
        let queried = unsafe {
            QueryInformationJobObject(
                self.job.as_raw_handle() as HANDLE,
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                size,
                ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(accounting.ActiveProcesses)
    }
}

/// A process identified by its Windows handle, which still refers to the
/// same process after it exits.
#[derive(Debug)]
pub struct ProcessHandle(OwnedHandle);

impl ProcessHandle {
    /// Retains a process for exit checks and requests termination rights when
    /// Windows grants them.
    ///
    /// # Errors
    ///
    /// The system's error if the process cannot be opened for an exit check.
    pub fn open(process_id: u32) -> io::Result<Self> {
        open_process(process_id, PROCESS_SYNCHRONIZE | PROCESS_TERMINATE)
            .or_else(|_| open_process(process_id, PROCESS_SYNCHRONIZE))
            .map(Self)
    }

    /// Returns whether the process has yet to signal its exit.
    #[must_use]
    pub fn is_running(&self) -> bool {
        // SAFETY: this owns a valid process handle with synchronize access.
        unsafe { WaitForSingleObject(self.0.as_raw_handle() as HANDLE, 0) != WAIT_OBJECT_0 }
    }

    /// Asks Windows to terminate the process if the handle permits it.
    pub fn terminate(&self) {
        // SAFETY: the process handle is valid. The caller checks for exit.
        unsafe { TerminateProcess(self.0.as_raw_handle() as HANDLE, 1) };
    }
}

/// Whether a process with this ID is still running.
#[must_use]
pub fn process_running(process_id: u32) -> bool {
    let Ok(process) = open_process(
        process_id,
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
    ) else {
        return false;
    };
    // SAFETY: the process handle is valid for the call.
    unsafe { WaitForSingleObject(process.as_raw_handle() as HANDLE, 0) == WAIT_TIMEOUT }
}

/// Ends a process by its ID. A process that has ended already succeeds.
pub fn terminate_process(process_id: u32) {
    if let Ok(process) = open_process(process_id, PROCESS_TERMINATE) {
        // SAFETY: the process handle is valid for the call. A failure means
        // that the process is ending or gone, which the caller checks.
        unsafe { TerminateProcess(process.as_raw_handle() as HANDLE, 1) };
    }
}

fn open_process(process_id: u32, access: u32) -> io::Result<OwnedHandle> {
    // SAFETY: OpenProcess takes plain values; the result is checked before
    // it is used.
    let raw = unsafe { OpenProcess(access, 0, process_id) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a new, valid handle that nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw as RawHandle) })
}

#[cfg(test)]
mod process_tree_job_tests {
    use super::{ProcessTreeJob, process_running};

    #[test]
    fn terminating_the_job_ends_a_process_that_joined_it_after_starting() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "ping -n 30 127.0.0.1 > nul"])
            .spawn()
            .unwrap();
        let job = ProcessTreeJob::new().unwrap();
        let process = job.adopt(child.id()).unwrap();
        // Assigning again is harmless.
        job.assign(child.id()).unwrap();
        assert!(process.is_running());
        assert!(job.active_processes().unwrap() >= 1);
        job.terminate().unwrap();
        child.wait().unwrap();
        assert!(!process.is_running());
        assert_eq!(job.active_processes().unwrap(), 0);
    }

    #[test]
    fn dropping_the_job_ends_its_processes() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "ping -n 30 127.0.0.1 > nul"])
            .spawn()
            .unwrap();
        let job = ProcessTreeJob::new().unwrap();
        job.assign(child.id()).unwrap();
        drop(job);
        child.wait().unwrap();
        assert!(!process_running(child.id()));
    }

    #[test]
    fn a_missing_process_cannot_join() {
        let job = ProcessTreeJob::new().unwrap();
        job.adopt(u32::MAX - 3).unwrap_err();
    }
}
