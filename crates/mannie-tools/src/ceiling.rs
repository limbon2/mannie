//! A worker's policy ceiling (plugins/workers/docs/host-and-ceiling.md, the
//! ceiling; ADR-0028): the most that its tools may do, as the owner set it in
//! the worker's configuration on its host, and the mode that a task narrowed
//! it to. The ceiling also decides the master's requests on the databases
//! that the owner registered on the host (ADR-0057, decision 5).

use std::path::{Path, PathBuf};

use mannie_types::ceiling::CeilingMode;
use mannie_types::ids::DatabaseKey;
use mannie_types::tools::{
    CommandRule, GrantScope, HostDatabaseRefusal, StatementClass, WriteAuthority,
};

use crate::digest::WrittenStatement;
use crate::paths::is_inside;

/// The ceiling that the policy applies to every call on a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ceiling {
    /// The mode: read-only, ask or autonomous.
    pub mode: CeilingMode,
    /// The folders in which reads run without asking, and changes too in
    /// autonomous mode. Canonical, as the accesses are.
    pub allowed_folders: Vec<PathBuf>,
    /// The host's command rules, which let a command run without asking in
    /// autonomous mode.
    pub rules: Vec<CommandRule>,
}

impl Ceiling {
    /// The ceiling for a task that asks for `narrower`: the same folders and
    /// rules, and a mode that is never wider than this one's.
    #[must_use]
    pub fn narrowed(&self, narrower: Option<CeilingMode>) -> Self {
        Self {
            mode: self.mode.narrowed_by(narrower),
            ..self.clone()
        }
    }
}

/// A request of the master on a database that the owner registered on this
/// host (ADR-0057, decision 5).
#[derive(Debug, Clone, Copy)]
pub struct DatabaseRequest<'a> {
    /// The canonical path of the registered file.
    pub path: &'a Path,
    /// The worker's own folders, in canonical form, where no registered
    /// file may be.
    pub protected: &'a [PathBuf],
    /// The write, or `None` for a read.
    pub write: Option<HostWrite<'a>>,
}

/// A write that the master asks of a database on this host.
#[derive(Debug, Clone, Copy)]
pub struct HostWrite<'a> {
    /// The database's key.
    pub key: &'a DatabaseKey,
    /// The write's statements, as this host classified them.
    pub statements: &'a [WrittenStatement<'a>],
    /// The write's digest, as this host computed it from its own registry
    /// and the write.
    pub digest: &'a str,
    /// What the write's evidence says lets it run.
    pub authority: &'a WriteAuthority,
}

/// What a host's ceiling decides about a request on one of its databases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostDecision {
    /// The request runs.
    Runs,
    /// The owner decides first, through the master: the write runs with the
    /// owner's Allow once of its digest.
    NeedsOwner,
    /// The ceiling refuses it, whatever the master sends.
    Refused(HostDatabaseRefusal),
}

/// Decides a request of the master on a database of this host (ADR-0057,
/// decision 5). A file inside the worker's own folders is refused in every
/// mode, and one outside the allowed folders unless the host is in full
/// access, so a file whose folder the configuration no longer allows stops
/// at once. A read then runs. A write or a change of a schema:
///
/// | Authority                    | Read-only | Ask        | Autonomous | Full access |
/// | ---------------------------- | --------- | ---------- | ---------- | ----------- |
/// | The owner's action           | Refused   | Runs       | Runs       | Runs        |
/// | The owner's Allow once       | Refused   | Runs       | Runs       | Runs        |
/// | A grant of the master        | Refused   | The owner  | Runs       | Runs        |
/// | The master's full access     | Refused   | Runs (1)   | Runs (1)   | Runs        |
///
/// (1) when the host accepts the master's full access, and the owner's
/// decision otherwise. An Allow once runs only when its digest is the one
/// that this host computed, and grants only when they cover every statement
/// on this database, which no change of a schema is; anything else goes to
/// the owner.
pub fn decide_database_request(
    ceiling: &Ceiling,
    accepts_master_full_access: bool,
    request: &DatabaseRequest<'_>,
) -> HostDecision {
    if request
        .protected
        .iter()
        .any(|folder| is_inside(request.path, folder))
    {
        return HostDecision::Refused(HostDatabaseRefusal::WorkerFolder);
    }
    let full_access = ceiling.mode == CeilingMode::FullAccess;
    let inside = ceiling
        .allowed_folders
        .iter()
        .any(|folder| is_inside(request.path, folder));
    if !inside && !full_access {
        return HostDecision::Refused(HostDatabaseRefusal::OutsideAllowedFolders);
    }
    let Some(write) = request.write else {
        return HostDecision::Runs;
    };
    let accepted = match ceiling.mode {
        CeilingMode::ReadOnly => {
            return HostDecision::Refused(HostDatabaseRefusal::ReadOnlyHost);
        }
        CeilingMode::FullAccess => true,
        CeilingMode::Ask | CeilingMode::Autonomous => match write.authority {
            WriteAuthority::OwnerAction => true,
            WriteAuthority::AllowedOnce { digest } => *digest == write.digest,
            WriteAuthority::Grants { scopes } => {
                ceiling.mode == CeilingMode::Autonomous && grants_cover(scopes, &write)
            }
            WriteAuthority::FullAccess => accepts_master_full_access,
        },
    };
    if accepted {
        HostDecision::Runs
    } else {
        HostDecision::NeedsOwner
    }
}

/// Whether grants cover every statement of a write: each a write, never a
/// change of a schema, whose text a grant of this database names. The
/// worker knows no plugins, so a grant's plugin is the master's to check.
fn grants_cover(scopes: &[GrantScope], write: &HostWrite<'_>) -> bool {
    let covered = |text: &str| {
        scopes.iter().any(|scope| match scope {
            GrantScope::Database { key, statement, .. } => key == write.key && statement == text,
            GrantScope::Path { .. }
            | GrantScope::Command { .. }
            | GrantScope::RemoteCommand { .. }
            | GrantScope::Program { .. }
            | GrantScope::ExternalTool { .. }
            | GrantScope::TerminalCommand { .. } => false,
        })
    };
    !write.statements.is_empty()
        && write
            .statements
            .iter()
            .all(|statement| statement.class == StatementClass::Write && covered(statement.text))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use mannie_types::ceiling::CeilingMode;
    use mannie_types::ids::DatabaseKey;
    use mannie_types::tools::{GrantScope, HostDatabaseRefusal, StatementClass, WriteAuthority};

    use super::{Ceiling, DatabaseRequest, HostDecision, HostWrite, decide_database_request};
    use crate::digest::WrittenStatement;

    const KEY: &str = "0199e0f2-0000-7000-8000-0000000000ee";
    const UPDATE: &str = "UPDATE parts SET stock = ?1";

    /// An absolute path on this platform.
    fn at(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        } else {
            PathBuf::from(path)
        }
    }

    fn ceiling(mode: CeilingMode) -> Ceiling {
        Ceiling {
            mode,
            allowed_folders: vec![at("/srv/app")],
            rules: Vec::new(),
        }
    }

    fn key() -> DatabaseKey {
        DatabaseKey::parse(KEY).unwrap()
    }

    fn grant(text: &str) -> GrantScope {
        GrantScope::Database {
            plugin: "workers".to_owned(),
            key: key(),
            name: "Inventory".to_owned(),
            statement: text.to_owned(),
        }
    }

    /// Decides a request on `/srv/app/parts.db`, or another file, with the
    /// worker's data in `/var/lib/mannie-worker`.
    struct Case {
        path: PathBuf,
        statements: Vec<WrittenStatement<'static>>,
        authority: Option<WriteAuthority>,
    }

    impl Case {
        fn read() -> Self {
            Self {
                path: at("/srv/app/parts.db"),
                statements: Vec::new(),
                authority: None,
            }
        }

        fn write(class: StatementClass, authority: WriteAuthority) -> Self {
            Self {
                path: at("/srv/app/parts.db"),
                statements: vec![WrittenStatement {
                    text: UPDATE,
                    parameters_digest: "aa",
                    class,
                }],
                authority: Some(authority),
            }
        }

        fn with_file(mut self, path: &str) -> Self {
            self.path = at(path);
            self
        }

        fn decide(&self, mode: CeilingMode, accepts: bool) -> HostDecision {
            let key = key();
            let protected = [at("/var/lib/mannie-worker")];
            let write = self.authority.as_ref().map(|authority| HostWrite {
                key: &key,
                statements: &self.statements,
                digest: "the host's digest",
                authority,
            });
            decide_database_request(
                &ceiling(mode),
                accepts,
                &DatabaseRequest {
                    path: Path::new(&self.path),
                    protected: &protected,
                    write,
                },
            )
        }
    }

    const MODES: [CeilingMode; 4] = [
        CeilingMode::ReadOnly,
        CeilingMode::Ask,
        CeilingMode::Autonomous,
        CeilingMode::FullAccess,
    ];

    fn allowed_once() -> WriteAuthority {
        WriteAuthority::AllowedOnce {
            digest: "the host's digest".to_owned(),
        }
    }

    #[test]
    fn a_read_runs_in_every_mode_inside_the_allowed_folders() {
        for mode in MODES {
            assert_eq!(
                Case::read().decide(mode, false),
                HostDecision::Runs,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn a_file_outside_the_allowed_folders_is_refused_unless_in_full_access() {
        for mode in [
            CeilingMode::ReadOnly,
            CeilingMode::Ask,
            CeilingMode::Autonomous,
        ] {
            assert_eq!(
                Case::read()
                    .with_file("/home/owner/parts.db")
                    .decide(mode, true),
                HostDecision::Refused(HostDatabaseRefusal::OutsideAllowedFolders),
                "{mode:?}"
            );
        }
        assert_eq!(
            Case::read()
                .with_file("/home/owner/parts.db")
                .decide(CeilingMode::FullAccess, false),
            HostDecision::Runs
        );
    }

    #[test]
    fn a_file_in_the_workers_own_folders_is_refused_in_every_mode() {
        for mode in MODES {
            assert_eq!(
                Case::write(StatementClass::Write, WriteAuthority::OwnerAction)
                    .with_file("/var/lib/mannie-worker/worker.db")
                    .decide(mode, true),
                HostDecision::Refused(HostDatabaseRefusal::WorkerFolder),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn a_read_only_host_refuses_every_write_whatever_its_authority() {
        for authority in [
            WriteAuthority::OwnerAction,
            allowed_once(),
            WriteAuthority::Grants {
                scopes: vec![grant(UPDATE)],
            },
            WriteAuthority::FullAccess,
        ] {
            for class in [StatementClass::Write, StatementClass::Schema] {
                assert_eq!(
                    Case::write(class, authority.clone()).decide(CeilingMode::ReadOnly, true),
                    HostDecision::Refused(HostDatabaseRefusal::ReadOnlyHost),
                    "{authority:?} {class:?}"
                );
            }
        }
    }

    #[test]
    fn the_owners_action_and_allow_once_run_on_every_host_that_writes() {
        for mode in [
            CeilingMode::Ask,
            CeilingMode::Autonomous,
            CeilingMode::FullAccess,
        ] {
            for class in [StatementClass::Write, StatementClass::Schema] {
                for authority in [WriteAuthority::OwnerAction, allowed_once()] {
                    assert_eq!(
                        Case::write(class, authority.clone()).decide(mode, false),
                        HostDecision::Runs,
                        "{mode:?} {class:?} {authority:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_allow_once_of_another_write_goes_back_to_the_owner() {
        let other = WriteAuthority::AllowedOnce {
            digest: "the digest that the master saw".to_owned(),
        };

        for mode in [CeilingMode::Ask, CeilingMode::Autonomous] {
            assert_eq!(
                Case::write(StatementClass::Write, other.clone()).decide(mode, true),
                HostDecision::NeedsOwner,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn a_grant_needs_the_owner_on_an_ask_host_and_runs_on_an_autonomous_one() {
        let granted = WriteAuthority::Grants {
            scopes: vec![grant(UPDATE)],
        };

        assert_eq!(
            Case::write(StatementClass::Write, granted.clone()).decide(CeilingMode::Ask, true),
            HostDecision::NeedsOwner
        );
        assert_eq!(
            Case::write(StatementClass::Write, granted.clone())
                .decide(CeilingMode::Autonomous, false),
            HostDecision::Runs
        );
        assert_eq!(
            Case::write(StatementClass::Write, granted).decide(CeilingMode::FullAccess, false),
            HostDecision::Runs
        );
    }

    #[test]
    fn grants_that_do_not_cover_the_write_go_to_the_owner() {
        let mut other_database = grant(UPDATE);
        if let GrantScope::Database { key, .. } = &mut other_database {
            *key = DatabaseKey::parse("0199e0f2-0000-7000-8000-0000000000ef").unwrap();
        }
        for scopes in [
            vec![grant("UPDATE parts SET stock = 0")],
            vec![other_database],
            Vec::new(),
        ] {
            assert_eq!(
                Case::write(StatementClass::Write, WriteAuthority::Grants { scopes })
                    .decide(CeilingMode::Autonomous, true),
                HostDecision::NeedsOwner
            );
        }
        // No grant covers a change of a schema, even one that names it.
        assert_eq!(
            Case::write(
                StatementClass::Schema,
                WriteAuthority::Grants {
                    scopes: vec![grant(UPDATE)]
                }
            )
            .decide(CeilingMode::Autonomous, true),
            HostDecision::NeedsOwner
        );
    }

    #[test]
    fn the_masters_full_access_runs_only_where_the_host_accepts_it() {
        for mode in [CeilingMode::Ask, CeilingMode::Autonomous] {
            for class in [StatementClass::Write, StatementClass::Schema] {
                assert_eq!(
                    Case::write(class, WriteAuthority::FullAccess).decide(mode, true),
                    HostDecision::Runs,
                    "{mode:?} {class:?}"
                );
                assert_eq!(
                    Case::write(class, WriteAuthority::FullAccess).decide(mode, false),
                    HostDecision::NeedsOwner,
                    "{mode:?} {class:?}"
                );
            }
        }
        assert_eq!(
            Case::write(StatementClass::Schema, WriteAuthority::FullAccess)
                .decide(CeilingMode::FullAccess, false),
            HostDecision::Runs
        );
    }
}
