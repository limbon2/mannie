//! Schema-scoped development databases (`task dev`).
//!
//! Development builds run under their own bundle identifier
//! (`io.github.limbon2.mannie.dev`) so they stay separate from the owner's
//! release database. Even within development, switching git branches, testing
//! older commits or editing an in-progress migration can change the module
//! schema versions or SQL. Sharing one SQLite file across branches then fails
//! with `ModuleTooNew` on the older branch or silently collides when two
//! branches add different migrations under the same version number.
//!
//! This module keeps one SQLite database per exact schema fingerprint inside
//! `<data_directory>/dev/<fingerprint>/mannie.sqlite`, beside a
//! `manifest.json` that records each module's applied migration digests and
//! last-used time. Release and validation builds continue to call
//! [`Store::open`] directly and enforce ADR-0009 without change.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use mannie_types::clock::UtcMillis;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{DatabaseError, StoreError};
use crate::plugin_store::PluginModule;
use crate::schema::{Records, Schema, StoreModule};
use crate::store::{BUSY_TIMEOUT, Store, StoreOptions, prepare_connection, unique_module_names};

/// Folder inside the development data directory that holds schema-scoped
/// databases.
const DEV_DIRECTORY: &str = "dev";

/// Database file name inside each schema folder and at the legacy root.
const DATABASE_FILE: &str = "mannie.sqlite";

/// Sidecar metadata beside each development database.
const MANIFEST_FILE: &str = "manifest.json";

/// Backup folder name inside each schema folder.
const BACKUPS_DIRECTORY: &str = "backups";

/// Prefix for temporary staging folders cleaned up on startup.
const STAGING_PREFIX: &str = ".staging-";

/// Manifest schema version.
const MANIFEST_VERSION: u32 = 1;

/// Hex characters used for each schema folder name (64 bits of SHA-256).
const FINGERPRINT_HEX_LEN: usize = 16;

/// Maximum number of development schema folders kept on disk.
const MAX_DEV_DATABASES: usize = 12;

/// Monotonic counter so concurrent staging folders in tests never collide.
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Deterministic identity of the active build's schema across all registered
/// core and plugin modules.
#[derive(Debug, Clone)]
struct SchemaIdentity {
    /// Short folder name under `dev/`.
    directory: String,
    /// Full SHA-256 hex digest of the schema.
    fingerprint: String,
    /// Ordered migration SQL SHA-256 digests for each module, sorted by name.
    modules: BTreeMap<String, Vec<String>>,
}

impl SchemaIdentity {
    fn from_modules(modules: &[StoreModule]) -> Self {
        let mut by_module: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for module in modules {
            let digests: Vec<String> = module
                .migrations
                .iter()
                .map(|migration| hash_migration(migration.version, migration.sql))
                .collect();
            by_module.insert(module.name.to_owned(), digests);
        }

        let mut hasher = Sha256::new();
        hasher.update(b"mannie-dev-schema-v1\0");

        for (name, digests) in &by_module {
            let name_len = u64::try_from(name.len()).unwrap_or(u64::MAX);
            let digest_count = u64::try_from(digests.len()).unwrap_or(u64::MAX);
            hasher.update(name_len.to_be_bytes());
            hasher.update(name.as_bytes());
            hasher.update(digest_count.to_be_bytes());

            for digest in digests {
                hasher.update(digest.as_bytes());
                hasher.update(b"\n");
            }
        }

        let fingerprint = hex_encode(&hasher.finalize());
        let directory = fingerprint[..FINGERPRINT_HEX_LEN].to_owned();

        Self {
            directory,
            fingerprint,
            modules: by_module,
        }
    }
}

/// Metadata written beside each schema-scoped development database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevManifest {
    version: u32,
    fingerprint: String,
    last_used_at: i64,
    modules: BTreeMap<String, Vec<String>>,
}

/// A compatible existing database that can seed a new schema folder.
#[derive(Debug)]
struct SeedCandidate {
    kind: SeedKind,
    database: PathBuf,
    matched_versions: u64,
    extra_modules: usize,
    recency: i64,
    tie_breaker: String,
}

#[derive(Debug)]
enum SeedKind {
    VerifiedDev(DevManifest),
    LegacyDatabase,
    LegacyBackup,
}

/// Temporary staging folder removed automatically if seeding fails before
/// atomic publish.
struct StagingDir {
    path: PathBuf,
    cleanup: bool,
}

impl StagingDir {
    fn create(dev_root: &Path) -> Result<Self, StoreError> {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!("{STAGING_PREFIX}{}-{sequence}", std::process::id());
        let path = dev_root.join(name);
        create_private_dir(&path)?;

        Ok(Self {
            path,
            cleanup: true,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.cleanup = false;
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        if self.cleanup
            && let Err(error) = fs::remove_dir_all(&self.path)
        {
            tracing::warn!(
                directory = %self.path.display(),
                %error,
                "could not remove temporary development database staging folder"
            );
        }
    }
}

/// Opens a schema-scoped development database under `data_directory/dev/`.
pub(crate) fn open(
    data_directory: &Path,
    options: &StoreOptions,
    modules: &[StoreModule],
    plugins: &[PluginModule],
) -> Result<Store, StoreError> {
    let _names = unique_module_names(modules)?;
    let identity = SchemaIdentity::from_modules(modules);
    let dev_root = data_directory.join(DEV_DIRECTORY);
    create_private_dir(&dev_root)?;
    clean_abandoned_staging(&dev_root);

    let target_dir = dev_root.join(&identity.directory);
    let target_db = target_dir.join(DATABASE_FILE);
    let target_options = schema_store_options(&target_dir, options);
    let now = options.clock.now();

    let already_exists = fs::exists(&target_db).map_err(|source| StoreError::Io {
        path: target_db.clone(),
        source,
    })?;

    if already_exists {
        let store = Store::open_registering(&target_db, &target_options, modules, plugins)?;
        refresh_manifest(&target_dir, &identity, now);
        prune_old_databases(&dev_root, &identity.directory, MAX_DEV_DATABASES);

        return Ok(store);
    }

    let candidates = discover_candidates(
        data_directory,
        &dev_root,
        &options.backup_directory,
        &identity,
        modules,
    );
    let mut seeded = false;

    for candidate in candidates {
        match stage_database(
            &dev_root,
            &target_dir,
            Some(&candidate),
            &identity,
            options,
            modules,
            now,
        ) {
            Ok(()) => {
                tracing::info!(
                    schema = %identity.directory,
                    seed = %candidate.database.display(),
                    "seeded development database for schema"
                );
                seeded = true;
                break;
            }
            Err(error) => {
                tracing::warn!(
                    schema = %identity.directory,
                    seed = %candidate.database.display(),
                    %error,
                    "skipping development database seed that could not be migrated"
                );
            }
        }
    }

    if !seeded {
        stage_database(
            &dev_root,
            &target_dir,
            None,
            &identity,
            options,
            modules,
            now,
        )?;
        tracing::info!(
            schema = %identity.directory,
            "created fresh development database for schema"
        );
    }

    let store = Store::open_registering(&target_db, &target_options, modules, plugins)?;
    prune_old_databases(&dev_root, &identity.directory, MAX_DEV_DATABASES);

    Ok(store)
}

/// Store options scoped to one development schema folder.
fn schema_store_options(target_dir: &Path, options: &StoreOptions) -> StoreOptions {
    let mut scoped = options.clone();
    scoped.backup_directory = target_dir.join(BACKUPS_DIRECTORY);
    scoped
}

/// Collects compatible seed candidates in strict priority order:
/// 1. Verified schema-scoped dev databases (matching migration SQL digests).
/// 2. The legacy `mannie.sqlite` at the root of the dev data directory.
/// 3. Pre-migration backups in `backup_directory`.
fn discover_candidates(
    data_directory: &Path,
    dev_root: &Path,
    backup_directory: &Path,
    identity: &SchemaIdentity,
    modules: &[StoreModule],
) -> Vec<SeedCandidate> {
    let mut verified = discover_verified_dev(dev_root, identity, modules);
    verified.sort_by(compare_candidates);

    let mut legacy_db = discover_legacy_database(data_directory, dev_root, modules);
    let mut backups = discover_legacy_backups(backup_directory, dev_root, modules);
    backups.sort_by(compare_candidates);

    let mut all = verified;
    all.append(&mut legacy_db);
    all.append(&mut backups);
    all
}

/// Orders candidates with the closest compatible schema first, then fewer
/// unrelated plugin modules, then most recently used.
fn compare_candidates(left: &SeedCandidate, right: &SeedCandidate) -> std::cmp::Ordering {
    right
        .matched_versions
        .cmp(&left.matched_versions)
        .then_with(|| left.extra_modules.cmp(&right.extra_modules))
        .then_with(|| right.recency.cmp(&left.recency))
        .then_with(|| right.tie_breaker.cmp(&left.tie_breaker))
}

fn discover_verified_dev(
    dev_root: &Path,
    identity: &SchemaIdentity,
    modules: &[StoreModule],
) -> Vec<SeedCandidate> {
    let Ok(entries) = fs::read_dir(dev_root) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();

    for entry in entries.flatten() {
        let directory = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        let is_skipped = name == identity.directory || name.starts_with(STAGING_PREFIX);

        if is_skipped {
            continue;
        }

        let is_directory = entry.file_type().is_ok_and(|kind| kind.is_dir());

        if !is_directory {
            continue;
        }

        let database = directory.join(DATABASE_FILE);

        if !database.is_file() {
            continue;
        }

        let Some(manifest) = read_manifest(&directory.join(MANIFEST_FILE)) else {
            continue;
        };

        let Ok(schema) = read_candidate_schema(&database, dev_root) else {
            continue;
        };

        if schema.refuse_newer(&database, modules).is_err() {
            continue;
        }

        let records = schema.current_records(modules);

        if !is_manifest_compatible(&manifest, &records, identity, modules) {
            continue;
        }

        let (matched_versions, extra_modules) = score_records(&records, modules);
        let recency = manifest.last_used_at;

        candidates.push(SeedCandidate {
            kind: SeedKind::VerifiedDev(manifest),
            database,
            matched_versions,
            extra_modules,
            recency,
            tie_breaker: name,
        });
    }

    candidates
}

fn discover_legacy_database(
    data_directory: &Path,
    dev_root: &Path,
    modules: &[StoreModule],
) -> Vec<SeedCandidate> {
    let database = data_directory.join(DATABASE_FILE);

    if !database.is_file() {
        return Vec::new();
    }

    let Ok(schema) = read_candidate_schema(&database, dev_root) else {
        return Vec::new();
    };

    if schema.refuse_newer(&database, modules).is_err() {
        return Vec::new();
    }

    let records = schema.current_records(modules);
    let (matched_versions, extra_modules) = score_records(&records, modules);

    vec![SeedCandidate {
        kind: SeedKind::LegacyDatabase,
        recency: modified_millis(&database),
        database,
        matched_versions,
        extra_modules,
        tie_breaker: DATABASE_FILE.to_owned(),
    }]
}

fn discover_legacy_backups(
    backup_directory: &Path,
    dev_root: &Path,
    modules: &[StoreModule],
) -> Vec<SeedCandidate> {
    let Ok(entries) = fs::read_dir(backup_directory) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let is_backup = path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("bak"));

        if !is_backup {
            continue;
        }

        let Ok(schema) = read_candidate_schema(&path, dev_root) else {
            continue;
        };

        if schema.refuse_newer(&path, modules).is_err() {
            continue;
        }

        let records = schema.current_records(modules);
        let (matched_versions, extra_modules) = score_records(&records, modules);
        let name = entry.file_name().to_string_lossy().into_owned();

        candidates.push(SeedCandidate {
            kind: SeedKind::LegacyBackup,
            recency: modified_millis(&path),
            database: path,
            matched_versions,
            extra_modules,
            tie_breaker: name,
        });
    }

    candidates
}

/// Verifies that every migration already applied to a known module in the seed
/// matches this build's exact migration SQL digest prefix.
fn is_manifest_compatible(
    manifest: &DevManifest,
    records: &Records,
    identity: &SchemaIdentity,
    modules: &[StoreModule],
) -> bool {
    for module in modules {
        let recorded = records.get(module.name).copied().unwrap_or(0);
        let Some(expected) = identity.modules.get(module.name) else {
            return false;
        };
        let Ok(recorded_len) = usize::try_from(recorded) else {
            return false;
        };

        if recorded_len > expected.len() {
            return false;
        }

        if recorded_len == 0 {
            let has_unexpected = manifest
                .modules
                .get(module.name)
                .is_some_and(|hashes| !hashes.is_empty());

            if has_unexpected {
                return false;
            }

            continue;
        }

        let Some(seed_hashes) = manifest.modules.get(module.name) else {
            return false;
        };

        if seed_hashes.len() < recorded_len {
            return false;
        }

        if seed_hashes[..recorded_len] != expected[..recorded_len] {
            return false;
        }
    }

    true
}

/// Computes `(matched_versions, extra_modules)` for candidate ranking.
fn score_records(records: &Records, modules: &[StoreModule]) -> (u64, usize) {
    let matched_versions: u64 = modules
        .iter()
        .map(|module| u64::from(records.get(module.name).copied().unwrap_or(0)))
        .sum();
    let extra_modules = records
        .iter()
        .filter(|(name, version)| {
            let has_migrations = **version > 0;
            let is_unknown = !modules.iter().any(|module| module.name == name.as_str());
            has_migrations && is_unknown
        })
        .count();

    (matched_versions, extra_modules)
}

/// Stages a new schema database (seeded or fresh), migrates and validates it,
/// writes its manifest and atomically renames the staging folder into place.
fn stage_database(
    dev_root: &Path,
    target_dir: &Path,
    seed: Option<&SeedCandidate>,
    identity: &SchemaIdentity,
    options: &StoreOptions,
    modules: &[StoreModule],
    now: UtcMillis,
) -> Result<(), StoreError> {
    let mut staging = StagingDir::create(dev_root)?;
    let staged_db = staging.path().join(DATABASE_FILE);

    if let Some(seed) = seed {
        snapshot_database(&seed.database, &staged_db)?;
    }

    let staged_options = schema_store_options(staging.path(), options);
    let (connection, _names) = prepare_connection(&staged_db, &staged_options, modules, false)?;

    if let Err(error) = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
        tracing::debug!(%error, "checkpoint of staged development database did not complete");
    }

    drop(connection);

    let manifest = manifest_for_target(seed, identity, now);
    write_manifest_atomic(staging.path(), &manifest)?;

    if target_dir.exists() {
        let has_database = target_dir.join(DATABASE_FILE).exists();

        if !has_database && let Err(error) = fs::remove_dir_all(target_dir) {
            tracing::warn!(
                directory = %target_dir.display(),
                %error,
                "could not remove empty development database folder before publishing"
            );
        }
    }

    fs::rename(staging.path(), target_dir).map_err(|source| StoreError::Io {
        path: target_dir.to_path_buf(),
        source,
    })?;
    staging.disarm();

    Ok(())
}

/// Reads a candidate's schema without modifying the candidate file. Falls back
/// to inspecting a temporary copy when a WAL database lacks its `-shm` file.
fn read_candidate_schema(path: &Path, dev_root: &Path) -> Result<Schema, StoreError> {
    let open_failed = |source: rusqlite::Error| StoreError::Open {
        path: path.to_path_buf(),
        source: DatabaseError(source),
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let direct = Connection::open_with_flags(path, flags).and_then(|connection| {
        connection.busy_timeout(BUSY_TIMEOUT)?;
        Schema::read(&connection)
    });

    match direct {
        Ok(schema) => Ok(schema),
        Err(error) => {
            if !wal_path(path).exists() {
                return Err(open_failed(error));
            }

            read_schema_from_wal_copy(path, dev_root)
        }
    }
}

/// Copies a WAL database and its `-wal` into a temporary folder so SQLite can
/// replay the log and read its schema without touching the original files.
fn read_schema_from_wal_copy(path: &Path, dev_root: &Path) -> Result<Schema, StoreError> {
    let staging = StagingDir::create(dev_root)?;
    let copy = staging.path().join(DATABASE_FILE);
    copy_database_files(path, &copy)?;

    let open_failed = |source: rusqlite::Error| StoreError::Open {
        path: path.to_path_buf(),
        source: DatabaseError(source),
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(&copy, flags).map_err(open_failed)?;
    connection.busy_timeout(BUSY_TIMEOUT).map_err(open_failed)?;

    let schema = Schema::read(&connection).map_err(open_failed)?;
    drop(connection);

    Ok(schema)
}

/// Copies `source` to `destination` without writing to `source`. Uses
/// `VACUUM INTO` from a read-only connection first so WAL changes are merged
/// into a single consistent file, falling back to copying the database and its
/// `-wal` when read-only WAL recovery is unavailable.
fn snapshot_database(source: &Path, destination: &Path) -> Result<(), StoreError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let target = destination.to_string_lossy().into_owned();
    let vacuum = Connection::open_with_flags(source, flags).and_then(|connection| {
        connection.busy_timeout(BUSY_TIMEOUT)?;
        connection.execute("VACUUM INTO ?1", [&target])
    });

    if vacuum.is_ok() {
        return Ok(());
    }

    if destination.exists()
        && let Err(error) = fs::remove_file(destination)
    {
        tracing::debug!(
            path = %destination.display(),
            %error,
            "could not remove partial staged database before file copy"
        );
    }

    copy_database_files(source, destination)
}

/// Copies `source` and any companion `-wal` file into `destination`, omitting
/// `-shm` so SQLite rebuilds the shared-memory index cleanly on open.
fn copy_database_files(source: &Path, destination: &Path) -> Result<(), StoreError> {
    fs::copy(source, destination).map_err(|source_error| StoreError::Io {
        path: destination.to_path_buf(),
        source: source_error,
    })?;

    let source_wal = wal_path(source);

    if source_wal.is_file() {
        let destination_wal = wal_path(destination);
        fs::copy(&source_wal, &destination_wal).map_err(|source_error| StoreError::Io {
            path: destination_wal,
            source: source_error,
        })?;
    }

    Ok(())
}

/// Builds the manifest for a newly staged schema folder, preserving verified
/// migration digests of any unknown modules carried over from a verified seed.
fn manifest_for_target(
    seed: Option<&SeedCandidate>,
    identity: &SchemaIdentity,
    now: UtcMillis,
) -> DevManifest {
    let mut modules = BTreeMap::new();

    if let Some(SeedCandidate {
        kind: SeedKind::VerifiedDev(seed_manifest),
        ..
    }) = seed
    {
        for (name, hashes) in &seed_manifest.modules {
            modules.insert(name.clone(), hashes.clone());
        }
    }

    for (name, hashes) in &identity.modules {
        modules.insert(name.clone(), hashes.clone());
    }

    DevManifest {
        version: MANIFEST_VERSION,
        fingerprint: identity.fingerprint.clone(),
        last_used_at: now.as_millis(),
        modules,
    }
}

/// Updates `last_used_at` on an existing schema folder's manifest, preserving
/// any carried-over unknown module entries. Failure is logged and never blocks
/// startup.
fn refresh_manifest(target_dir: &Path, identity: &SchemaIdentity, now: UtcMillis) {
    let manifest_path = target_dir.join(MANIFEST_FILE);
    let mut modules = read_manifest(&manifest_path)
        .map(|existing| existing.modules)
        .unwrap_or_default();

    for (name, hashes) in &identity.modules {
        modules.insert(name.clone(), hashes.clone());
    }

    let updated = DevManifest {
        version: MANIFEST_VERSION,
        fingerprint: identity.fingerprint.clone(),
        last_used_at: now.as_millis(),
        modules,
    };

    if let Err(error) = write_manifest_atomic(target_dir, &updated) {
        tracing::warn!(
            path = %manifest_path.display(),
            %error,
            "could not update development database manifest"
        );
    }
}

fn read_manifest(path: &Path) -> Option<DevManifest> {
    let bytes = fs::read(path).ok()?;
    let manifest: DevManifest = serde_json::from_slice(&bytes).ok()?;

    if manifest.version != MANIFEST_VERSION {
        return None;
    }

    Some(manifest)
}

fn write_manifest_atomic(directory: &Path, manifest: &DevManifest) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|source| StoreError::Io {
        path: directory.join(MANIFEST_FILE),
        source: std::io::Error::other(source),
    })?;

    let temporary = directory.join(format!("{MANIFEST_FILE}.tmp"));
    let final_path = directory.join(MANIFEST_FILE);

    fs::write(&temporary, bytes).map_err(|source| StoreError::Io {
        path: temporary.clone(),
        source,
    })?;
    restrict_file_permissions(&temporary)?;

    fs::rename(&temporary, &final_path).map_err(|source| StoreError::Io {
        path: final_path,
        source,
    })
}

/// Removes leftover staging folders from interrupted startups.
fn clean_abandoned_staging(dev_root: &Path) {
    let Ok(entries) = fs::read_dir(dev_root) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_staging = name.starts_with(STAGING_PREFIX);

        if !is_staging {
            continue;
        }

        let path = entry.path();

        if let Err(error) = fs::remove_dir_all(&path) {
            tracing::warn!(
                directory = %path.display(),
                %error,
                "could not remove abandoned development staging folder"
            );
        }
    }
}

/// Keeps the newest `max_kept` schema folders (always including `active`).
fn prune_old_databases(dev_root: &Path, active: &str, max_kept: usize) {
    let Ok(entries) = fs::read_dir(dev_root) else {
        return;
    };

    let mut others: Vec<(i64, String, PathBuf)> = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_skipped = name == active || name.starts_with(STAGING_PREFIX);

        if is_skipped {
            continue;
        }

        let is_directory = entry.file_type().is_ok_and(|kind| kind.is_dir());

        if !is_directory {
            continue;
        }

        let path = entry.path();
        let has_store_files =
            path.join(DATABASE_FILE).exists() || path.join(MANIFEST_FILE).exists();

        if !has_store_files {
            continue;
        }

        let last_used = read_manifest(&path.join(MANIFEST_FILE)).map_or_else(
            || modified_millis(&path.join(DATABASE_FILE)),
            |manifest| manifest.last_used_at,
        );
        others.push((last_used, name, path));
    }

    others.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));

    let keep_others = max_kept.saturating_sub(1);

    for (_, _, old_directory) in others.into_iter().skip(keep_others) {
        if let Err(error) = fs::remove_dir_all(&old_directory) {
            tracing::warn!(
                directory = %old_directory.display(),
                %error,
                "could not prune old development database folder"
            );
        }
    }
}

fn hash_migration(version: u32, sql: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(version.to_be_bytes());
    hasher.update(b"\0");
    hasher.update(sql.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        write!(text, "{byte:02x}").expect("writing to a String cannot fail");
    }

    text
}

fn wal_path(database: &Path) -> PathBuf {
    let mut path = OsString::from(database.as_os_str());
    path.push("-wal");
    PathBuf::from(path)
}

fn modified_millis(path: &Path) -> i64 {
    let latest = [path.to_path_buf(), wal_path(path)]
        .into_iter()
        .filter_map(|file| fs::metadata(file).ok()?.modified().ok())
        .max();

    latest
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn create_private_dir(path: &Path) -> Result<(), StoreError> {
    fs::create_dir_all(path).map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            StoreError::Io {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }

    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the Unix version of this function can fail"
)]
fn restrict_file_permissions(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}
