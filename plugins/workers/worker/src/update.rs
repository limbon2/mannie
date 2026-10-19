//! `mannie-worker update` (plugins/workers/docs/compatibility-and-updates.md,
//! updating a worker; decision D20): the owner installs a signed release on
//! the host. The archive is read once into a private copy and checked
//! against the release key built into this program; the new program must be
//! for this system and must answer `self-check`; the service stops, the
//! database files are copied, and the new program takes the place of the old
//! one, which stays beside it. The new program has a minute to write its
//! health marker; otherwise the old program and the database copy come back.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use base64::Engine as _;

use crate::cli::UpdateArguments;
use crate::health;
use crate::paths::{Folders, PREVIOUS_PROGRAM_FILE, PROGRAM_FILE};
use crate::run;
use crate::self_check::{self, SelfCheck};
use crate::service::{ServiceControl, SystemService};

/// The most bytes of a release archive.
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;

/// The most bytes of a signature file.
const MAX_SIGNATURE_BYTES: u64 = 4 * 1024;

/// How long the new program has to write its health marker.
const HEALTH_TIMEOUT: Duration = Duration::from_mins(1);

/// How long `self-check` of the new program may take.
const SELF_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// How often an update looks for the health marker.
const POLL: Duration = Duration::from_millis(250);

/// The database files, which an update copies while the service is stopped.
const DATABASE_FILES: [&str; 3] = [
    "mannie-worker.sqlite",
    "mannie-worker.sqlite-wal",
    "mannie-worker.sqlite-shm",
];

/// The folder of the database copy in the data folder.
const BACKUP_FOLDER: &str = "update-backup";

/// What an update does on the host, which the tests stand in for.
pub(crate) trait UpdateHost {
    /// The installed service.
    fn service(&self) -> &dyn ServiceControl;
    /// Unpacks `archive` into the empty folder `folder`.
    fn unpack(&self, archive: &Path, folder: &Path) -> anyhow::Result<()>;
    /// Runs `program self-check --json` and reads its answer.
    fn inspect(&self, program: &Path) -> anyhow::Result<SelfCheck>;
}

/// An update of the program in `program_folder`, whose data is in
/// `data_folder`.
#[derive(Debug, Clone)]
pub(crate) struct Update<'key> {
    pub(crate) program_folder: PathBuf,
    pub(crate) data_folder: PathBuf,
    /// The raw Ed25519 key that release archives are signed with.
    pub(crate) release_key: &'key [u8],
    /// The check of the program that runs the update.
    pub(crate) running: SelfCheck,
    /// How long the new program has to write its health marker.
    pub(crate) health_timeout: Duration,
}

/// What an update installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Installed {
    /// The version that runs now.
    pub(crate) version: String,
    /// Where the previous program stays.
    pub(crate) previous: PathBuf,
}

/// The signature file of an archive: its name with `.sig` added.
fn signature_path(archive: &Path) -> PathBuf {
    let mut name = archive.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

/// Reads at most `limit` bytes of a file, and fails when it is longer.
fn read_limited(path: &Path, limit: u64, what: &str) -> anyhow::Result<Vec<u8>> {
    let file = fs::File::open(path)
        .with_context(|| format!("could not open the {what} {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("could not read the {what} {}", path.display()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        bail!("the {what} {} is larger than {limit} bytes", path.display());
    }
    Ok(bytes)
}

impl Update<'_> {
    /// Reads the archive once and checks its signature against the release
    /// key. The bytes returned are the ones that were checked, so a change of
    /// the file afterwards does not reach the installed program.
    pub(crate) fn verified(&self, archive: &Path) -> anyhow::Result<Vec<u8>> {
        let bytes = read_limited(archive, MAX_ARCHIVE_BYTES, "archive")?;
        let signature_file = signature_path(archive);
        let text =
            read_limited(&signature_file, MAX_SIGNATURE_BYTES, "signature").with_context(|| {
                format!(
                    "an archive needs its signature, {}, from the release beside it",
                    signature_file.display()
                )
            })?;
        let text = String::from_utf8(text).context("the signature is not text")?;
        let signature = base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .context("the signature is not base64")?;
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, self.release_key)
            .verify(&bytes, &signature)
            .map_err(|_invalid| {
                anyhow::anyhow!(
                    "the signature of {} is not valid for Mannie's release key; the archive was changed or does not come from a release",
                    archive.display()
                )
            })?;
        Ok(bytes)
    }

    /// Installs the signed archive and keeps the new version once it wrote
    /// its health marker; otherwise the previous program and the database
    /// copy come back.
    pub(crate) fn install(
        &self,
        archive: &Path,
        host: &dyn UpdateHost,
    ) -> anyhow::Result<Installed> {
        let bytes = self.verified(archive)?;
        let work = tempfile::Builder::new()
            .prefix("mannie-worker-update-")
            .tempdir()
            .context("could not create a private folder for the update")?;
        let file_name = archive
            .file_name()
            .context("the archive has no file name")?;
        let copy = work.path().join(file_name);
        fs::write(&copy, &bytes).context("could not copy the archive into the private folder")?;
        let unpacked = work.path().join("unpacked");
        fs::create_dir(&unpacked).context("could not create the folder to unpack into")?;
        host.unpack(&copy, &unpacked)?;
        let new_program = unpacked.join(PROGRAM_FILE);
        if !new_program.is_file() {
            bail!("the archive has no {PROGRAM_FILE}; it is not a worker's archive");
        }
        let check = host.inspect(&new_program)?;
        self.check_target(&check)?;
        let service = host.service();
        if !service.is_installed()? {
            bail!(
                "the worker is not installed as a service. Install it with `mannie-worker service install`"
            );
        }
        if service.is_running()? {
            service.stop().context("could not stop the service")?;
            say("Stopped the service.");
        }
        self.copy_database()?;
        let installed = Installed {
            previous: self.swap_program(&new_program)?,
            version: check.version.clone(),
        };
        say(&format!("Installed version {}.", check.version));
        match self.start_and_wait(service, &check.version) {
            Ok(()) => {
                self.forget_database_copy();
                Ok(installed)
            }
            Err(failure) => {
                let restored = self.roll_back(service);
                match restored {
                    Ok(()) => Err(failure.context(
                        "the new version did not start; the previous program and its database are back and running",
                    )),
                    Err(error) => Err(failure.context(format!(
                        "the new version did not start, and putting the previous program back failed too: {error:#}. The previous program is {}, and the database copy is in {}",
                        installed.previous.display(),
                        self.data_folder.join(BACKUP_FOLDER).display()
                    ))),
                }
            }
        }
    }

    /// Refuses a program for another system, and warns about an older one.
    fn check_target(&self, check: &SelfCheck) -> anyhow::Result<()> {
        if (check.os.as_str(), check.arch.as_str())
            != (self.running.os.as_str(), self.running.arch.as_str())
        {
            bail!(
                "the archive's program is for {}-{}, and this host runs {}-{}. Download the archive for this system",
                check.os,
                check.arch,
                self.running.os,
                self.running.arch
            );
        }
        if version_parts(&check.version) < version_parts(&self.running.version) {
            say(&format!(
                "Warning: version {} is older than the installed version {}. A database that the newer version changed may not open; the update then restores this version.",
                check.version, self.running.version
            ));
        }
        if check.release_key_sha256 != self.running.release_key_sha256 {
            say("Warning: the new program checks updates with another release key than this one.");
        }
        Ok(())
    }

    /// Copies the database files while the service is stopped.
    fn copy_database(&self) -> anyhow::Result<()> {
        let backup = self.data_folder.join(BACKUP_FOLDER);
        match fs::remove_dir_all(&backup) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("could not clear {}", backup.display()));
            }
        }
        fs::create_dir(&backup)
            .with_context(|| format!("could not create {}", backup.display()))?;
        for name in DATABASE_FILES {
            let file = self.data_folder.join(name);
            if file.is_file() {
                fs::copy(&file, backup.join(name))
                    .with_context(|| format!("could not copy {}", file.display()))?;
            }
        }
        Ok(())
    }

    /// Puts the new program in place and keeps the old one beside it, whose
    /// path it returns. The copy goes to a temporary name first, so the
    /// folder never holds half a program.
    fn swap_program(&self, new_program: &Path) -> anyhow::Result<PathBuf> {
        let program = self.program_folder.join(PROGRAM_FILE);
        let previous = self.program_folder.join(PREVIOUS_PROGRAM_FILE);
        let staged = self.program_folder.join(format!("{PROGRAM_FILE}.new"));
        fs::copy(new_program, &staged)
            .with_context(|| format!("could not copy the new program to {}", staged.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
                .with_context(|| format!("could not make {} executable", staged.display()))?;
        }
        match fs::remove_file(&previous) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not replace {}", previous.display()));
            }
        }
        fs::rename(&program, &previous).with_context(|| {
            format!(
                "could not keep the previous program as {}",
                previous.display()
            )
        })?;
        fs::rename(&staged, &program)
            .with_context(|| format!("could not put the new program at {}", program.display()))?;
        Ok(previous)
    }

    /// Starts the service and waits for the health marker of `version`.
    fn start_and_wait(&self, service: &dyn ServiceControl, version: &str) -> anyhow::Result<()> {
        health::remove(&self.data_folder).context("could not remove the old health marker")?;
        service.start().context("the service did not start")?;
        let deadline = Instant::now() + self.health_timeout;
        loop {
            if health::read(&self.data_folder).is_some_and(|health| health.version == version) {
                say(&format!("Version {version} runs."));
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "version {version} did not report that it runs within {} seconds; its log in {} says why",
                    self.health_timeout.as_secs(),
                    self.data_folder.join("logs").display()
                );
            }
            thread::sleep(POLL);
        }
    }

    /// Stops the service, puts the previous program and the database copy
    /// back, and starts the service again.
    fn roll_back(&self, service: &dyn ServiceControl) -> anyhow::Result<()> {
        if service.is_running().unwrap_or(true) {
            service.stop().context("could not stop the new version")?;
        }
        let program = self.program_folder.join(PROGRAM_FILE);
        let previous = self.program_folder.join(PREVIOUS_PROGRAM_FILE);
        fs::remove_file(&program)
            .with_context(|| format!("could not remove the new program {}", program.display()))?;
        fs::rename(&previous, &program).with_context(|| {
            format!(
                "could not put the previous program back at {}",
                program.display()
            )
        })?;
        self.restore_database()?;
        service
            .start()
            .context("the previous version did not start again")?;
        say("Put the previous program and its database back, and started it.");
        Ok(())
    }

    /// Puts the database copy back: the files that were copied replace the
    /// ones there, and the files that were not are removed.
    fn restore_database(&self) -> anyhow::Result<()> {
        let backup = self.data_folder.join(BACKUP_FOLDER);
        for name in DATABASE_FILES {
            let kept = backup.join(name);
            let file = self.data_folder.join(name);
            if kept.is_file() {
                fs::copy(&kept, &file)
                    .with_context(|| format!("could not restore {}", file.display()))?;
                #[cfg(unix)]
                crate::access::give_to_owner_of(&file, &self.data_folder).with_context(|| {
                    format!(
                        "could not give {} back to the worker's account",
                        file.display()
                    )
                })?;
            } else {
                match fs::remove_file(&file) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("could not remove {}", file.display()));
                    }
                }
            }
        }
        self.forget_database_copy();
        Ok(())
    }

    /// Deletes the database copy, which holds the conversations of the tasks.
    fn forget_database_copy(&self) {
        let backup = self.data_folder.join(BACKUP_FOLDER);
        if let Err(error) = fs::remove_dir_all(&backup) {
            say(&format!(
                "Warning: could not delete the database copy {} ({error}); delete it by hand.",
                backup.display()
            ));
        }
    }
}

/// The numbers of a version, compared part by part. Text after a hyphen,
/// as in a pre-release, is left out.
fn version_parts(version: &str) -> Vec<u64> {
    version
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// The host as it is: the system's service manager, its `tar`, and the new
/// program itself.
struct SystemHost;

impl UpdateHost for SystemHost {
    fn service(&self) -> &dyn ServiceControl {
        &SystemService
    }

    fn unpack(&self, archive: &Path, folder: &Path) -> anyhow::Result<()> {
        let tar = system_tar()?;
        // bsdtar on Windows and FreeBSD, and GNU tar on Linux, recognize the
        // archive's format and compression themselves, zip included.
        let output = Command::new(&tar)
            .arg("-xf")
            .arg(archive)
            .arg("-C")
            .arg(folder)
            .output()
            .with_context(|| format!("could not run {}", tar.display()))?;
        if !output.status.success() {
            bail!(
                "could not unpack the archive with {} ({}): {}",
                tar.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn inspect(&self, program: &Path) -> anyhow::Result<SelfCheck> {
        let mut child = Command::new(program)
            .args(["self-check", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("the new program does not run on this host")?;
        let deadline = Instant::now() + SELF_CHECK_TIMEOUT;
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .context("could not wait for the new program")?
            {
                break status;
            }
            if Instant::now() >= deadline {
                let _killed = child.kill();
                let _reaped = child.wait();
                bail!(
                    "the new program did not answer `self-check` within {} seconds",
                    SELF_CHECK_TIMEOUT.as_secs()
                );
            }
            thread::sleep(Duration::from_millis(50));
        };
        if !status.success() {
            bail!("the new program's `self-check` failed ({status})");
        }
        let mut output = String::new();
        child
            .stdout
            .take()
            .context("the new program's output is missing")?
            .read_to_string(&mut output)
            .context("could not read the new program's `self-check`")?;
        serde_json::from_str(output.trim()).context("the new program's `self-check` is not valid")
    }
}

/// The system's `tar`, from its fixed place, never from the search path.
fn system_tar() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    let candidates = {
        let windows = std::env::var_os("SystemRoot")
            .context("the SystemRoot environment variable is missing")?;
        vec![Path::new(&windows).join("System32").join("tar.exe")]
    };
    #[cfg(unix)]
    let candidates = vec![PathBuf::from("/usr/bin/tar"), PathBuf::from("/bin/tar")];
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .context("this system has no tar program in its usual place")
}

/// Runs `update`: checks the archive, and installs it unless `--check`.
pub(crate) fn update(arguments: &UpdateArguments) -> anyhow::Result<()> {
    let folders = Folders::of_this_system()?;
    let running = self_check::this_program();
    let mut update = Update {
        program_folder: folders.program.clone(),
        data_folder: folders.data.clone(),
        release_key: self_check::release_key(),
        running,
        health_timeout: HEALTH_TIMEOUT,
    };
    if arguments.check {
        update.verified(&arguments.archive)?;
        say(&format!(
            "The signature of {} is valid for Mannie's release key.",
            arguments.archive.display()
        ));
        return Ok(());
    }
    update.data_folder = run::installed_configuration(&arguments.config, &folders)?.data_dir;
    update.install(&arguments.archive, &SystemHost)?;
    say(&format!(
        "The update is done. The previous program stays as {}.",
        folders.program.join(PREVIOUS_PROGRAM_FILE).display()
    ));
    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "update reports each step on the console"
)]
fn say(text: &str) {
    println!("{text}");
}

#[cfg(test)]
mod tests;
