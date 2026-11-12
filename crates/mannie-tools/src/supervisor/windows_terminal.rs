//! The Windows half of a terminal's shell (ADR-0053, decision 1): a pseudo
//! console from `CreatePseudoConsole`, and the shell in a Job Object of its
//! own, as every command is (ADR-0014, decision 2).
//!
//! The standard library cannot start a process with a pseudo console, so
//! the shell starts through `CreateProcessW` itself: suspended, with the
//! pseudo console as an attribute, then put into its job before its thread
//! runs. Closing the pseudo console hangs up the shell; terminating the job
//! ends what is left, such as a program that the shell started in a console
//! of its own.
#![expect(
    unsafe_code,
    reason = "a terminal's shell needs Win32 functions that windows-sys exposes as unsafe (ADR-0053)"
)]

use std::ffi::{OsStr, OsString, c_void};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::Path;
use std::ptr;
use std::sync::{Arc, Once};
use std::time::Duration;

use mannie_types::tools::ProcessRecord;
use tokio::task::JoinHandle;
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
    SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::terminal::TerminalLaunch;
use super::windows::{assign, create_job, creation_time, terminate};

/// A shell in a pseudo console, with the job that holds its processes.
#[derive(Debug)]
pub(super) struct Pty {
    console: HPCON,
    job: OwnedHandle,
    exited: Option<JoinHandle<Option<i32>>>,
    exit: ShellExit,
    record: ProcessRecord,
}

/// What the wait for the shell learned.
#[derive(Debug, Clone, Copy)]
enum ShellExit {
    /// Nobody has waited for the shell to exit yet.
    Waiting,
    /// The shell exited with this code.
    Exited(i32),
    /// The shell ended, and the system gave no exit code.
    Unreadable,
}

impl Pty {
    /// Starts the shell in a new pseudo console of the launch's size, and
    /// returns it with the console's output to read and its input to write.
    pub(super) fn spawn(launch: &TerminalLaunch) -> io::Result<(Self, File, File)> {
        clear_ignored_ctrl_c();
        let (input_read, input_write) = pipe()?;
        let (output_read, output_write) = pipe()?;
        let mut console: HPCON = 0;
        let size = coordinates(launch.columns, launch.rows);
        // SAFETY: the two handles are valid pipe ends that live until the
        // call returns, and `console` is a writable value; the result is
        // checked before the console is used.
        let created = unsafe {
            CreatePseudoConsole(
                size,
                input_read.as_raw_handle() as HANDLE,
                output_write.as_raw_handle() as HANDLE,
                0,
                &raw mut console,
            )
        };
        if created < 0 {
            return Err(io::Error::from_raw_os_error(created));
        }
        // The pseudo console holds its own copies of these ends; this
        // process keeps the other two.
        drop(input_read);
        drop(output_write);
        match start_shell(launch, console) {
            Ok((job, process, record)) => {
                let process = Arc::new(process);
                let exited = tokio::task::spawn_blocking(move || wait_for(&process));
                let pty = Self {
                    console,
                    job,
                    exited: Some(exited),
                    exit: ShellExit::Waiting,
                    record,
                };
                Ok((pty, File::from(output_read), File::from(input_write)))
            }
            Err(error) => {
                // SAFETY: the console was created above and is closed once.
                unsafe { ClosePseudoConsole(console) };
                Err(error)
            }
        }
    }

    /// The shell's process.
    pub(super) fn record(&self) -> ProcessRecord {
        self.record.clone()
    }

    /// Changes the pseudo console's size.
    pub(super) fn resize(&self, columns: u16, rows: u16) -> io::Result<()> {
        // SAFETY: the console stays open until `end`, which takes `self`.
        let resized = unsafe { ResizePseudoConsole(self.console, coordinates(columns, rows)) };
        if resized < 0 {
            return Err(io::Error::from_raw_os_error(resized));
        }
        Ok(())
    }

    /// Waits until the shell exits, and returns its exit code.
    pub(super) async fn finished(&mut self) -> Option<i32> {
        match self.exit {
            ShellExit::Exited(code) => return Some(code),
            ShellExit::Unreadable => return None,
            ShellExit::Waiting => {}
        }
        let exited = self.exited.as_mut()?;
        let code = exited.await.ok().flatten();
        self.exited = None;
        self.exit = code.map_or(ShellExit::Unreadable, ShellExit::Exited);
        code
    }

    /// Closes the pseudo console, which hangs up the shell, waits at most
    /// `grace` for it to exit, then ends every process left in the job.
    pub(super) async fn end(mut self, grace: Duration) -> Option<i32> {
        let console = self.console;
        // Closing waits until the console's host has drained what it wrote,
        // which the reading thread keeps doing, so it runs on a blocking
        // thread of its own.
        let closing = tokio::task::spawn_blocking(move || {
            // SAFETY: the console was created in `spawn` and is closed once:
            // `end` takes `self`, and `Drop` does not close it.
            unsafe { ClosePseudoConsole(console) };
        });
        let exited = tokio::time::timeout(grace, self.finished())
            .await
            .ok()
            .flatten();
        terminate(&self.job);
        self.finished().await;
        if let Err(error) = closing.await {
            tracing::warn!(%error, "the pseudo console of a terminal was not closed");
        }
        exited
    }
}

/// Starts the shell suspended with the pseudo console, puts it into a new
/// job, and only then lets it run.
fn start_shell(
    launch: &TerminalLaunch,
    console: HPCON,
) -> io::Result<(OwnedHandle, OwnedHandle, ProcessRecord)> {
    let job = create_job()?;
    let mut attributes = AttributeList::with_console(console)?;
    // SAFETY: the structure is plain data, for which all zeros is a valid
    // value.
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb =
        u32::try_from(size_of::<STARTUPINFOEXW>()).map_err(io::Error::other)?;
    // A parent whose own standard handles are redirected, as Mannie's may be,
    // would otherwise hand them to the shell instead of the pseudo console.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    startup.lpAttributeList = attributes.as_mut_ptr();
    let mut command_line = command_line(&launch.program, &launch.arguments);
    let environment = environment_block(&launch.environment);
    let directory = wide(launch.working_directory.as_os_str());
    // SAFETY: the structure is plain data, for which all zeros is a valid
    // value.
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer refers to a value that lives until the call
    // returns: the command line is writable and NUL-terminated, the
    // environment block ends with two NULs, the folder is NUL-terminated,
    // and the start-up information holds the attribute list, which lives in
    // `attributes`. Handles are not inherited.
    let started = unsafe {
        CreateProcessW(
            ptr::null(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
            environment.as_ptr().cast::<c_void>(),
            directory.as_ptr(),
            &raw const startup.StartupInfo,
            &raw mut info,
        )
    };
    if started == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both handles are new and valid, and nothing else owns them.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess as RawHandle) };
    // SAFETY: as above.
    let thread = unsafe { OwnedHandle::from_raw_handle(info.hThread as RawHandle) };
    let joined = assign(&job, process.as_raw_handle()).and_then(|()| resume(&thread));
    if let Err(error) = joined {
        // The shell is still suspended: end it before it runs anything.
        terminate(&job);
        return Err(error);
    }
    let record = ProcessRecord {
        pid: info.dwProcessId,
        start_time: creation_time(process.as_raw_handle()).unwrap_or_default(),
    };
    Ok((job, process, record))
}

/// A list of process attributes that holds one attribute, the pseudo
/// console.
struct AttributeList {
    buffer: Vec<usize>,
}

impl AttributeList {
    fn with_console(console: HPCON) -> io::Result<Self> {
        let mut size = 0_usize;
        // SAFETY: a null list asks only for the size it needs, which the
        // call writes into `size`; it fails by design.
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &raw mut size) };
        // A buffer of machine words keeps the list aligned.
        let words = size.div_ceil(size_of::<usize>());
        let mut list = Self {
            buffer: vec![0; words],
        };
        // SAFETY: the buffer holds at least `size` bytes, as the call asked.
        let initialized =
            unsafe { InitializeProcThreadAttributeList(list.as_mut_ptr(), 1, 0, &raw mut size) };
        if initialized == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the list was initialized for one attribute. The pseudo
        // console's attribute takes the console's value itself as its value,
        // as Windows documents, and the console outlives the process start.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                list.as_mut_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                console as *const c_void,
                size_of::<HPCON>(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if updated == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(list)
    }

    fn as_mut_ptr(&mut self) -> *mut c_void {
        self.buffer.as_mut_ptr().cast()
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: the list was initialized in `with_console` and is deleted
        // once, after the process started.
        unsafe { DeleteProcThreadAttributeList(self.as_mut_ptr()) };
    }
}

/// A pipe whose ends are not inherited.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read: HANDLE = ptr::null_mut();
    let mut write: HANDLE = ptr::null_mut();
    // SAFETY: both pointers refer to writable handles; null attributes make
    // the ends not inheritable, and the default size is used.
    let created = unsafe { CreatePipe(&raw mut read, &raw mut write, ptr::null(), 0) };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the handle is new and valid, and nothing else owns it.
    let read_end = unsafe { OwnedHandle::from_raw_handle(read as RawHandle) };
    // SAFETY: the same holds for the pipe's other end.
    let write_end = unsafe { OwnedHandle::from_raw_handle(write as RawHandle) };
    Ok((read_end, write_end))
}

/// Resumes the shell's only thread, which `CreateProcessW` handed out.
fn resume(thread: &OwnedHandle) -> io::Result<()> {
    // SAFETY: the thread handle is valid and has every right, as the
    // creator's handle does.
    let previous = unsafe { ResumeThread(thread.as_raw_handle() as HANDLE) };
    if previous == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Waits for the shell to exit on a blocking thread, and reads its code.
fn wait_for(process: &OwnedHandle) -> Option<i32> {
    // SAFETY: the process handle is valid while `process` lives.
    let waited = unsafe { WaitForSingleObject(process.as_raw_handle() as HANDLE, INFINITE) };
    if waited != WAIT_OBJECT_0 {
        return None;
    }
    let mut code = 0_u32;
    // SAFETY: the handle is valid and `code` is writable.
    let got = unsafe { GetExitCodeProcess(process.as_raw_handle() as HANDLE, &raw mut code) };
    // Exit codes such as `0xC000013A` read as negative numbers, as the
    // standard library reads them.
    (got != 0).then_some(code.cast_signed())
}

/// A child inherits whether its parent ignores Ctrl+C. A shell started from
/// a process that ignores it never gets the Ctrl+C that its terminal types,
/// so its commands could not be interrupted (S1): Mannie clears the flag once,
/// before its first terminal.
fn clear_ignored_ctrl_c() {
    static CLEARED: Once = Once::new();
    CLEARED.call_once(|| {
        // SAFETY: a null handler with `FALSE` only clears the process's flag
        // that ignores Ctrl+C; it registers nothing.
        let cleared = unsafe { SetConsoleCtrlHandler(None, 0) };
        if cleared == 0 {
            tracing::warn!(
                error = %io::Error::last_os_error(),
                "Mannie could not stop ignoring Ctrl+C, so its terminals may not interrupt commands"
            );
        }
    });
}

/// A size in the pseudo console's terms, within its limits.
fn coordinates(columns: u16, rows: u16) -> COORD {
    let clamp = |value: u16| i16::try_from(value.max(1)).unwrap_or(i16::MAX);
    COORD {
        X: clamp(columns),
        Y: clamp(rows),
    }
}

/// A NUL-terminated wide string.
pub(super) fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// The command line of the shell and its arguments, NUL-terminated, quoted
/// for the parsing that Windows programs share (`CommandLineToArgvW`).
pub(super) fn command_line(program: &Path, arguments: &[OsString]) -> Vec<u16> {
    let mut line: Vec<u16> = Vec::new();
    append_argument(&mut line, program.as_os_str());
    for argument in arguments {
        line.push(u16::from(b' '));
        append_argument(&mut line, argument);
    }
    line.push(0);
    line
}

/// Appends one argument: as it is when it holds no space, tab or quote, and
/// otherwise in quotes, with the backslashes before a quote and at the end
/// doubled.
fn append_argument(line: &mut Vec<u16>, argument: &OsStr) {
    let units: Vec<u16> = argument.encode_wide().collect();
    let space = u16::from(b' ');
    let tab = u16::from(b'\t');
    let quote = u16::from(b'"');
    let backslash = u16::from(b'\\');
    let plain = !units.is_empty() && !units.iter().any(|unit| [space, tab, quote].contains(unit));
    if plain {
        line.extend_from_slice(&units);
        return;
    }
    line.push(quote);
    let mut backslashes = 0_usize;
    for &unit in &units {
        if unit == backslash {
            backslashes += 1;
            continue;
        }
        let doubled = if unit == quote {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        line.extend(std::iter::repeat_n(backslash, doubled));
        backslashes = 0;
        line.push(unit);
    }
    line.extend(std::iter::repeat_n(backslash, backslashes * 2));
    line.push(quote);
}

/// The environment as `CreateProcessW` takes it: `NAME=value` entries, each
/// NUL-terminated, sorted by name without regard to case as Windows keeps
/// them, then one more NUL.
pub(super) fn environment_block(environment: &[(OsString, OsString)]) -> Vec<u16> {
    let mut sorted: Vec<&(OsString, OsString)> = environment.iter().collect();
    sorted.sort_by_key(|(name, _value)| name.to_string_lossy().to_uppercase());
    let mut block: Vec<u16> = Vec::new();
    for (name, value) in sorted {
        block.extend(name.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use super::{command_line, environment_block};

    fn text(units: &[u16]) -> String {
        String::from_utf16(units.strip_suffix(&[0]).unwrap_or(units)).unwrap()
    }

    #[test]
    fn arguments_are_quoted_only_where_windows_programs_need_it() {
        let arguments = [
            OsString::from("-NoExit"),
            OsString::from("-Command"),
            OsString::from("$a = 'b c'; Write-Output $a"),
            OsString::from(r"C:\path with space\"),
            OsString::from(r#"say "hi""#),
            OsString::from(""),
        ];

        let line = text(&command_line(
            Path::new(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            &arguments,
        ));

        assert_eq!(
            line,
            r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoExit -Command "$a = 'b c'; Write-Output $a" "C:\path with space\\" "say \"hi\"" """#
        );
    }

    #[test]
    fn the_environment_block_is_sorted_and_ends_with_two_nuls() {
        let block = environment_block(&[
            (OsString::from("TERM"), OsString::from("xterm-256color")),
            (OsString::from("Path"), OsString::from(r"C:\Windows")),
            (OsString::from("COLORTERM"), OsString::from("truecolor")),
        ]);

        let entries: Vec<String> = block
            .split(|unit| *unit == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf16(entry).unwrap())
            .collect();

        assert_eq!(
            entries,
            [
                "COLORTERM=truecolor",
                r"Path=C:\Windows",
                "TERM=xterm-256color"
            ]
        );
        assert_eq!(&block[block.len() - 2..], &[0, 0]);
        assert_eq!(environment_block(&[]), vec![0, 0]);
    }
}
