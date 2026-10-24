//! Programs that run as servers (ADR-0043, decision 5): a server's input
//! stays open while it runs and reaches it in order, its output arrives as
//! bytes, a closed input ends a server that reads to the end, and when its
//! owner ends it or drops it, every process that it started ends with it, as
//! a command's do (ADR-0014, decision 8).
//!
//! The tests run the system's shell as the server's program, with a script
//! that echoes each line of its input.

#![cfg(test)]
#![cfg(any(windows, target_os = "linux", target_os = "freebsd"))]

use std::path::PathBuf;
use std::time::Duration;

use mannie_tools::{
    CommandEnvironment, ServerProcess, ServerProgram, ServerProgramError, ServerSpec,
    find_server_program,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// A folder for a test, in canonical form.
fn folder() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let path = PathBuf::from(mannie_tools::canonical_folder(temp.path()).unwrap());
    (temp, path)
}

/// The system's shell as a server that runs `script`.
fn shell_server(folder: &std::path::Path, script: &str) -> ServerSpec {
    #[cfg(windows)]
    let (program, arguments) = {
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap();
        (
            system.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
            vec![
                "-NoLogo".to_owned(),
                "-NoProfile".to_owned(),
                "-NonInteractive".to_owned(),
                "-Command".to_owned(),
                script.to_owned(),
            ],
        )
    };
    #[cfg(not(windows))]
    let (program, arguments) = (
        PathBuf::from("/bin/sh"),
        vec!["-c".to_owned(), script.to_owned()],
    );
    ServerSpec {
        program,
        arguments,
        working_directory: folder.to_path_buf(),
        environment: CommandEnvironment::from_process(&[]).variables().to_vec(),
    }
}

/// A script that echoes each line of its input, prefixed, until its input
/// ends, and then exits with 7.
#[cfg(windows)]
const ECHO: &str = "$in=[Console]::In; while(($line=$in.ReadLine()) -ne $null){[Console]::Out.WriteLine('echo:'+$line);[Console]::Out.Flush()}; exit 7";
#[cfg(not(windows))]
const ECHO: &str = "while IFS= read -r line; do printf 'echo:%s\\n' \"$line\"; done; exit 7";

#[tokio::test]
async fn a_server_reads_its_input_in_order_and_answers_on_its_output() {
    let (_temp, folder) = folder();
    let mut server = ServerProcess::spawn(&shell_server(&folder, ECHO)).unwrap();
    let mut input = server.take_input().unwrap();
    let mut output = BufReader::new(server.take_output().unwrap());

    input.write_all(b"first\nsecond\n").await.unwrap();
    input.flush().await.unwrap();
    let mut lines = Vec::new();
    for _ in 0..2 {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(30), output.read_line(&mut line))
            .await
            .expect("the server answers")
            .unwrap();
        lines.push(line.trim_end().to_owned());
    }

    assert_eq!(lines, ["echo:first", "echo:second"]);
    drop(input);
    let code = tokio::time::timeout(Duration::from_secs(30), server.finished())
        .await
        .expect("a server ends when its input closes");
    assert_eq!(code, Some(7));
    server.end(Duration::ZERO).await;
}

#[tokio::test]
async fn ending_a_server_closes_its_input_and_returns_its_exit_code() {
    let (_temp, folder) = folder();
    let server = ServerProcess::spawn(&shell_server(&folder, ECHO)).unwrap();

    let code = server.end(Duration::from_secs(30)).await;

    assert_eq!(code, Some(7));
}

#[tokio::test]
async fn ending_a_server_that_ignores_its_input_ends_its_whole_tree() {
    let (_temp, folder) = folder();
    let marker = folder.join("still-running");
    #[cfg(windows)]
    let script = format!(
        "Start-Process -NoNewWindow -FilePath powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep 3; Set-Content -Path ''{}'' -Value 1'; Start-Sleep 600",
        marker.display()
    );
    #[cfg(not(windows))]
    let script = format!("(sleep 3; echo 1 > '{}') & sleep 600", marker.display());
    let mut server = ServerProcess::spawn(&shell_server(&folder, &script)).unwrap();
    let mut errors = server.take_errors().unwrap();

    let code = server.end(Duration::from_millis(200)).await;
    tokio::time::sleep(Duration::from_secs(5)).await;

    assert_eq!(code, None);
    assert!(
        !marker.exists(),
        "a process of the server's tree outlived it"
    );
    let mut rest = Vec::new();
    let _read = errors.read_to_end(&mut rest).await;
}

#[tokio::test]
async fn a_dropped_server_ends_its_tree() {
    let (_temp, folder) = folder();
    let marker = folder.join("still-running");
    #[cfg(windows)]
    let script = format!(
        "Start-Sleep 3; Set-Content -Path '{}' -Value 1",
        marker.display()
    );
    #[cfg(not(windows))]
    let script = format!("sleep 3; echo 1 > '{}'", marker.display());
    let server = ServerProcess::spawn(&shell_server(&folder, &script)).unwrap();

    drop(server);
    tokio::time::sleep(Duration::from_secs(5)).await;

    assert!(!marker.exists(), "a dropped server kept running");
}

/// A `.cmd` file that prints its first argument as it received it, with its
/// quotes, and exits.
#[cfg(windows)]
fn batch_file(folder: &std::path::Path) -> PathBuf {
    let path = folder.join("server.cmd");
    std::fs::write(
        &path,
        "@echo off
echo started %1
",
    )
    .unwrap();
    path
}

/// What a batch file printed for one argument, or the kind of the error
/// that refused the argument before anything ran.
#[cfg(windows)]
async fn batch_output(
    folder: &std::path::Path,
    argument: &str,
) -> Result<String, std::io::ErrorKind> {
    let spec = ServerSpec {
        program: batch_file(folder),
        arguments: vec![argument.to_owned()],
        working_directory: folder.to_path_buf(),
        environment: CommandEnvironment::from_process(&[]).variables().to_vec(),
    };
    let mut server = ServerProcess::spawn(&spec).map_err(|error| error.kind())?;
    let mut output = server.take_output().unwrap();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(30), output.read_to_string(&mut text))
        .await
        .expect("the batch file ends")
        .unwrap();
    assert_eq!(server.end(Duration::from_secs(5)).await, Some(0));
    Ok(text)
}

#[cfg(windows)]
#[tokio::test]
async fn a_batch_file_starts_through_cmd_with_each_argument_quoted() {
    let (_temp, folder) = folder();

    let plain = batch_output(&folder, "hello world").await.unwrap();
    let injected = batch_output(&folder, "a&echo injected").await.unwrap();

    assert_eq!(plain.trim(), "started \"hello world\"");
    // The argument stays one quoted argument, so cmd.exe runs nothing of it.
    assert_eq!(injected.trim(), "started \"a&echo injected\"");
}

#[cfg(windows)]
#[tokio::test]
async fn an_argument_that_cmd_cannot_take_quoted_is_refused_before_anything_runs() {
    let (_temp, folder) = folder();
    let mut refused = Vec::new();
    for argument in [
        "a|b",
        "%PATH%",
        "a\"b",
        "a^b",
        "a<b",
        "line
break",
    ] {
        if let Err(kind) = batch_output(&folder, argument).await {
            refused.push((argument, kind));
        }
    }

    assert_eq!(
        refused,
        [(
            "line
break",
            std::io::ErrorKind::InvalidInput
        )]
    );
}

/// A file that counts as a program where a server looks for one: any bytes
/// on Windows, which decides by the file's kind, and an executable file
/// elsewhere.
fn program_file(path: &std::path::Path) {
    std::fs::write(path, b"program").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A `PATH` of these folders.
fn path_of(folders: &[&std::path::Path]) -> std::ffi::OsString {
    std::env::join_paths(folders).unwrap()
}

#[cfg(windows)]
#[test]
fn a_name_finds_an_exe_before_a_batch_file_and_never_a_powershell_script() {
    let (_first, first) = folder();
    let (_second, second) = folder();
    program_file(&first.join("tool.cmd"));
    program_file(&second.join("tool.exe"));
    program_file(&first.join("script.ps1"));
    program_file(&first.join("npx.cmd"));
    let path = path_of(&[&first, &second]);

    let tool = find_server_program("tool", &path, &[], &[]).unwrap();
    let npx = find_server_program("npx", &path, &[], &[]).unwrap();
    let named = find_server_program("npx.cmd", &path, &[], &[]).unwrap();

    assert_eq!(tool.path, second.join("tool.exe"));
    assert!(!tool.in_project);
    assert_eq!(npx.path, first.join("npx.cmd"));
    assert_eq!(named.path, first.join("npx.cmd"));
    assert_eq!(
        find_server_program("script", &path, &[], &[]),
        Err(ServerProgramError::NotFound)
    );
    assert_eq!(
        find_server_program(
            &first.join("script.ps1").display().to_string(),
            &path,
            &[],
            &[]
        ),
        Err(ServerProgramError::Refused)
    );
}

#[test]
fn a_name_skips_the_projects_and_mannies_folders_on_the_path() {
    let (_project, project) = folder();
    let (_mannie, mannie) = folder();
    let file = if cfg!(windows) { "tool.exe" } else { "tool" };
    program_file(&project.join(file));
    program_file(&mannie.join(file));
    let path = path_of(&[&project, &mannie]);

    assert_eq!(
        find_server_program(
            "tool",
            &path,
            std::slice::from_ref(&project),
            std::slice::from_ref(&mannie)
        ),
        Err(ServerProgramError::NotFound)
    );
}

#[test]
fn an_absolute_program_in_a_project_is_found_and_marked_and_one_in_mannies_folders_refused() {
    let (_project, project) = folder();
    let (_mannie, mannie) = folder();
    let file = if cfg!(windows) {
        "server.exe"
    } else {
        "server"
    };
    program_file(&project.join(file));
    program_file(&mannie.join(file));
    let in_project = project.join(file).display().to_string();
    let in_mannie = mannie.join(file).display().to_string();

    let found = find_server_program(
        &in_project,
        &path_of(&[]),
        std::slice::from_ref(&project),
        std::slice::from_ref(&mannie),
    )
    .unwrap();

    assert_eq!(
        found,
        ServerProgram {
            path: project.join(file),
            in_project: true
        }
    );
    assert_eq!(
        find_server_program(
            &in_mannie,
            &path_of(&[]),
            std::slice::from_ref(&project),
            std::slice::from_ref(&mannie)
        ),
        Err(ServerProgramError::Refused)
    );
}

#[test]
fn a_relative_or_network_program_is_refused_and_a_missing_one_not_found() {
    let path = path_of(&[]);
    for relative in ["bin/tool", "./tool", "..\\tool.exe"] {
        assert_eq!(
            find_server_program(relative, &path, &[], &[]),
            Err(ServerProgramError::Refused),
            "{relative}"
        );
    }
    #[cfg(windows)]
    assert_eq!(
        find_server_program(r"\\server\share\tool.exe", &path, &[], &[]),
        Err(ServerProgramError::Refused)
    );
    assert_eq!(
        find_server_program("no-such-program-4e1f", &path, &[], &[]),
        Err(ServerProgramError::NotFound)
    );
}
