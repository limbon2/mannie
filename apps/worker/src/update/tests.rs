use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use tempfile::TempDir;

use super::{Update, UpdateHost};
use crate::health::{HEALTH_FILE, Health};
use crate::paths::{PREVIOUS_PROGRAM_FILE, PROGRAM_FILE};
use crate::self_check::SelfCheck;
use crate::service::ServiceControl;

const DATABASE: &str = "mannie-worker.sqlite";

fn check(version: &str) -> SelfCheck {
    SelfCheck {
        version: version.to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        release_key_sha256: "ab".repeat(32),
    }
}

/// How the new program behaves when the stand-in service starts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Behavior {
    /// It writes its health marker.
    Starts,
    /// It changes the database, then exits without a marker.
    ChangesTheDatabaseAndFails,
}

/// The service manager, which runs whatever program is in the program
/// folder: the old one writes its marker, and the new one behaves as told.
struct StandInService {
    program_folder: PathBuf,
    data_folder: PathBuf,
    behavior: Behavior,
    running: Cell<bool>,
    starts: RefCell<Vec<String>>,
}

impl ServiceControl for StandInService {
    fn is_installed(&self) -> anyhow::Result<bool> {
        Ok(true)
    }

    fn is_running(&self) -> anyhow::Result<bool> {
        Ok(self.running.get())
    }

    fn stop(&self) -> anyhow::Result<()> {
        self.running.set(false);
        Ok(())
    }

    fn start(&self) -> anyhow::Result<()> {
        let program = fs::read_to_string(self.program_folder.join(PROGRAM_FILE))?;
        self.starts.borrow_mut().push(program.clone());
        self.running.set(true);
        let version = program.trim_start_matches("program ");
        let is_new = version == "0.5.0";
        if is_new && self.behavior == Behavior::ChangesTheDatabaseAndFails {
            fs::write(self.data_folder.join(DATABASE), "migrated by 0.5.0")?;
            self.running.set(false);
            return Ok(());
        }
        let health = Health {
            version: version.to_owned(),
            process_id: 1,
        };
        fs::write(
            self.data_folder.join(HEALTH_FILE),
            serde_json::to_vec(&health)?,
        )?;
        Ok(())
    }
}

/// The host: unpacking writes the new program, and inspecting it answers
/// with `answer`.
struct StandInHost {
    service: StandInService,
    answer: SelfCheck,
    archive_has_program: bool,
}

impl UpdateHost for StandInHost {
    fn service(&self) -> &dyn ServiceControl {
        &self.service
    }

    fn unpack(&self, archive: &Path, folder: &Path) -> anyhow::Result<()> {
        assert!(archive.is_file(), "the private copy is unpacked");
        if self.archive_has_program {
            fs::write(
                folder.join(PROGRAM_FILE),
                format!("program {}", self.answer.version),
            )?;
        }
        fs::write(folder.join("LICENSE"), "license")?;
        Ok(())
    }

    fn inspect(&self, _program: &Path) -> anyhow::Result<SelfCheck> {
        Ok(self.answer.clone())
    }
}

/// An installed worker of version 0.4.0, and a signed archive of 0.5.0.
struct Setup {
    _root: TempDir,
    program_folder: PathBuf,
    data_folder: PathBuf,
    archive: PathBuf,
    key: Vec<u8>,
}

impl Setup {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let program_folder = root.path().join("program");
        let data_folder = root.path().join("data");
        fs::create_dir_all(&program_folder).unwrap();
        fs::create_dir_all(&data_folder).unwrap();
        fs::write(program_folder.join(PROGRAM_FILE), "program 0.4.0").unwrap();
        fs::write(data_folder.join(DATABASE), "database of 0.4.0").unwrap();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let archive = root.path().join("mannie-worker-0.5.0.tar.gz");
        let content = b"the archive of 0.5.0";
        fs::write(&archive, content).unwrap();
        let signature = base64::engine::general_purpose::STANDARD.encode(pair.sign(content));
        fs::write(
            root.path().join("mannie-worker-0.5.0.tar.gz.sig"),
            format!("{signature}\n"),
        )
        .unwrap();
        Self {
            _root: root,
            program_folder,
            data_folder,
            archive,
            key: pair.public_key().as_ref().to_vec(),
        }
    }

    fn update(&self) -> Update<'_> {
        Update {
            program_folder: self.program_folder.clone(),
            data_folder: self.data_folder.clone(),
            release_key: &self.key,
            running: check("0.4.0"),
            health_timeout: Duration::from_millis(300),
        }
    }

    fn host(&self, behavior: Behavior, answer: SelfCheck) -> StandInHost {
        StandInHost {
            service: StandInService {
                program_folder: self.program_folder.clone(),
                data_folder: self.data_folder.clone(),
                behavior,
                running: Cell::new(true),
                starts: RefCell::new(Vec::new()),
            },
            answer,
            archive_has_program: true,
        }
    }

    fn program(&self) -> String {
        fs::read_to_string(self.program_folder.join(PROGRAM_FILE)).unwrap()
    }

    fn database(&self) -> String {
        fs::read_to_string(self.data_folder.join(DATABASE)).unwrap()
    }
}

#[test]
fn a_signed_archive_passes_the_check_and_a_changed_one_does_not() {
    let setup = Setup::new();

    setup.update().verified(&setup.archive).unwrap();
    fs::write(&setup.archive, b"the archive of 0.5.0, changed").unwrap();
    let changed = setup.update().verified(&setup.archive).unwrap_err();

    assert!(
        changed
            .to_string()
            .contains("not valid for Mannie's release key"),
        "{changed:#}"
    );
}

#[test]
fn an_archive_without_its_signature_or_with_another_key_is_refused() {
    let setup = Setup::new();
    let other = Ed25519KeyPair::from_pkcs8(
        Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    let mut update = setup.update();
    update.release_key = other.public_key().as_ref();

    let other_key = update.verified(&setup.archive).unwrap_err();
    fs::remove_file(super::signature_path(&setup.archive)).unwrap();
    let unsigned = setup.update().verified(&setup.archive).unwrap_err();

    assert!(other_key.to_string().contains("not valid"), "{other_key:#}");
    assert!(
        format!("{unsigned:#}").contains("needs its signature"),
        "{unsigned:#}"
    );
}

#[test]
fn a_new_version_that_starts_replaces_the_program_and_keeps_the_previous() {
    let setup = Setup::new();
    let host = setup.host(Behavior::Starts, check("0.5.0"));

    let installed = setup.update().install(&setup.archive, &host).unwrap();

    assert_eq!(installed.version, "0.5.0");
    assert_eq!(setup.program(), "program 0.5.0");
    assert_eq!(
        fs::read_to_string(setup.program_folder.join(PREVIOUS_PROGRAM_FILE)).unwrap(),
        "program 0.4.0"
    );
    assert_eq!(*host.service.starts.borrow(), ["program 0.5.0"]);
    assert!(
        !setup.data_folder.join(super::BACKUP_FOLDER).exists(),
        "the database copy is deleted"
    );
}

#[test]
fn a_new_version_that_does_not_start_is_rolled_back_with_its_database() {
    let setup = Setup::new();
    let host = setup.host(Behavior::ChangesTheDatabaseAndFails, check("0.5.0"));

    let failed = setup.update().install(&setup.archive, &host).unwrap_err();

    assert!(
        format!("{failed:#}").contains("previous program and its database are back"),
        "{failed:#}"
    );
    assert_eq!(setup.program(), "program 0.4.0");
    assert_eq!(setup.database(), "database of 0.4.0");
    assert!(!setup.program_folder.join(PREVIOUS_PROGRAM_FILE).exists());
    assert_eq!(
        *host.service.starts.borrow(),
        ["program 0.5.0", "program 0.4.0"],
        "the previous version runs again"
    );
}

#[test]
fn a_program_for_another_system_is_refused_before_anything_changes() {
    let setup = Setup::new();
    let mut answer = check("0.5.0");
    answer.os = "plan9".to_owned();
    let host = setup.host(Behavior::Starts, answer);

    let refused = setup.update().install(&setup.archive, &host).unwrap_err();

    assert!(refused.to_string().contains("for plan9"), "{refused:#}");
    assert_eq!(setup.program(), "program 0.4.0");
    assert!(host.service.running.get(), "the service was not stopped");
}

#[test]
fn an_archive_without_the_program_is_refused() {
    let setup = Setup::new();
    let mut host = setup.host(Behavior::Starts, check("0.5.0"));
    host.archive_has_program = false;

    let refused = setup.update().install(&setup.archive, &host).unwrap_err();

    assert!(
        refused.to_string().contains("not a worker's archive"),
        "{refused:#}"
    );
    assert_eq!(setup.program(), "program 0.4.0");
}

#[test]
fn versions_compare_by_their_numbers() {
    assert!(super::version_parts("0.10.0") > super::version_parts("0.9.3"));
    assert_eq!(
        super::version_parts("0.4.0-rc.1"),
        super::version_parts("0.4.0")
    );
    assert!(super::version_parts("1.0.0") > super::version_parts("0.99.99"));
}
