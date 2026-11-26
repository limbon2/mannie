//! SQLite persistence for the Proxmox plugin (`proxmox_servers` and
//! `proxmox_allowed_vms`). Passwords never live in SQLite (ADR-0010, ADR-0080).
//! Deleted rows erase freed database pages (`erase = true`, ADR-0048).

use std::collections::HashMap;
use std::sync::Arc;

use mannie_plugin_sdk::contracts::UtcMillis;
use mannie_plugin_sdk::services::store::{
    Constraint, Durability, Expect, Migration, ModuleStoreApi, Row, Statement, StoreError,
    StoreModule, WriteBatch,
};

use crate::model::{AllowedVmEntry, AllowedVmRecord, ServerRecord, ServerState};

/// Prefix of all tables owned by the Proxmox plugin.
pub(crate) const PREFIX: &str = "proxmox";

/// Constraint error code returned when inserting or renaming to a taken server name.
pub(crate) const NAME_TAKEN: &str = "name_taken";

const MIGRATIONS: &[Migration] = &[Migration::new(
    1,
    "create proxmox_servers and proxmox_allowed_vms",
    "CREATE TABLE proxmox_servers (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        host TEXT NOT NULL,
        port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
        username TEXT NOT NULL,
        password_set INTEGER NOT NULL CHECK (password_set IN (0, 1)),
        pinned_cert_sha256 TEXT,
        pinned_cert_subject TEXT,
        pinned_cert_at INTEGER,
        note TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        CONSTRAINT proxmox_servers_pin_complete CHECK (
            (pinned_cert_sha256 IS NULL) = (pinned_cert_subject IS NULL)
            AND (pinned_cert_subject IS NULL) = (pinned_cert_at IS NULL)
        )
    ) STRICT;

    CREATE TABLE proxmox_allowed_vms (
        server_id TEXT NOT NULL REFERENCES proxmox_servers(id) ON DELETE CASCADE,
        vmid INTEGER NOT NULL CHECK (vmid > 0),
        node TEXT NOT NULL,
        name TEXT NOT NULL,
        added_at INTEGER NOT NULL,
        PRIMARY KEY (server_id, vmid)
    ) STRICT;",
)];

const SELECT_SERVERS: &str = "SELECT id, name, host, port, username, password_set, \
    pinned_cert_sha256, pinned_cert_subject, pinned_cert_at, note \
    FROM proxmox_servers ORDER BY name COLLATE NOCASE, id";

const SELECT_ALLOWED_VMS: &str = "SELECT server_id, vmid, node, name, added_at \
    FROM proxmox_allowed_vms ORDER BY server_id, vmid";

const INSERT_SERVER: &str = "INSERT INTO proxmox_servers \
    (id, name, host, port, username, password_set, note, created_at, updated_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?7)";

const SET_PASSWORD_SET: &str =
    "UPDATE proxmox_servers SET password_set = ?2, updated_at = ?3 WHERE id = ?1";

const UPDATE_SERVER: &str = "UPDATE proxmox_servers \
    SET name = ?2, host = ?3, port = ?4, username = ?5, note = ?6, updated_at = ?7 \
    WHERE id = ?1";

const SET_PINNED_CERT: &str = "UPDATE proxmox_servers \
    SET pinned_cert_sha256 = ?2, pinned_cert_subject = ?3, pinned_cert_at = ?4, updated_at = ?4 \
    WHERE id = ?1";

const CLEAR_PINNED_CERT: &str = "UPDATE proxmox_servers \
    SET pinned_cert_sha256 = NULL, pinned_cert_subject = NULL, pinned_cert_at = NULL, \
    updated_at = ?2 WHERE id = ?1";

const DELETE_SERVER: &str = "DELETE FROM proxmox_servers WHERE id = ?1";

const DELETE_ALLOWED_VMS_FOR_SERVER: &str = "DELETE FROM proxmox_allowed_vms WHERE server_id = ?1";

const INSERT_ALLOWED_VM: &str = "INSERT INTO proxmox_allowed_vms \
    (server_id, vmid, node, name, added_at) VALUES (?1, ?2, ?3, ?4, ?5)";

const UPDATE_ALLOWED_VM_METADATA: &str = "UPDATE proxmox_allowed_vms \
    SET node = ?3, name = ?4 WHERE server_id = ?1 AND vmid = ?2";

/// The plugin's store module registered with the core database.
pub const MODULE: StoreModule = StoreModule::new(
    PREFIX,
    MIGRATIONS,
    &[
        SELECT_SERVERS,
        SELECT_ALLOWED_VMS,
        INSERT_SERVER,
        SET_PASSWORD_SET,
        UPDATE_SERVER,
        SET_PINNED_CERT,
        CLEAR_PINNED_CERT,
        DELETE_SERVER,
        DELETE_ALLOWED_VMS_FOR_SERVER,
        INSERT_ALLOWED_VM,
        UPDATE_ALLOWED_VM_METADATA,
    ],
);

/// Editable fields for inserting or updating a Proxmox server row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerFields {
    pub(crate) name: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) note: String,
}

/// Typed wrapper over the plugin's `ModuleStoreApi`.
#[derive(Debug, Clone)]
pub(crate) struct ProxmoxStore {
    store: Arc<dyn ModuleStoreApi>,
}

impl ProxmoxStore {
    pub(crate) fn new(store: Arc<dyn ModuleStoreApi>) -> Self {
        Self { store }
    }

    /// Loads every server along with its explicitly allowed VMs.
    pub(crate) async fn load_all(&self) -> Result<Vec<ServerState>, StoreError> {
        let server_rows = self.store.query(Statement::new(SELECT_SERVERS)).await?;
        let vm_rows = self.store.query(Statement::new(SELECT_ALLOWED_VMS)).await?;

        let mut vms_by_server: HashMap<String, Vec<AllowedVmRecord>> = HashMap::new();
        for row in &vm_rows {
            let record = parse_allowed_vm(row)?;
            vms_by_server
                .entry(record.server_id.clone())
                .or_default()
                .push(record);
        }

        let mut states = Vec::with_capacity(server_rows.len());
        for row in &server_rows {
            let server = parse_server(row)?;
            let allowed_vms = vms_by_server.remove(&server.id).unwrap_or_default();
            states.push(ServerState {
                server,
                allowed_vms,
            });
        }

        Ok(states)
    }

    /// Inserts a new server with `password_set = 0` and an empty VM allowlist.
    pub(crate) async fn insert_server(
        &self,
        id: &str,
        fields: &ServerFields,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let insert = Statement::new(INSERT_SERVER)
            .bind(id)
            .bind(fields.name.as_str())
            .bind(fields.host.as_str())
            .bind(i64::from(fields.port))
            .bind(fields.username.as_str())
            .bind(fields.note.as_str())
            .bind(now.as_millis());
        let batch = WriteBatch::new()
            .push(insert, Expect::Changes(1))
            .on_constraint(name_constraint(), NAME_TAKEN);
        self.write(batch, false).await
    }

    /// Updates `password_set` after saving or deleting the credential store entry.
    pub(crate) async fn set_password_set(
        &self,
        id: &str,
        password_set: bool,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let update = Statement::new(SET_PASSWORD_SET)
            .bind(id)
            .bind(password_set)
            .bind(now.as_millis());
        self.write(WriteBatch::new().push(update, Expect::Changes(1)), false)
            .await
    }

    /// Updates server metadata. When `clear_trust_and_allowlist` is true
    /// (because host or port changed), atomically clears both the pinned leaf
    /// certificate and every allowed VM row on this server.
    pub(crate) async fn update_server(
        &self,
        id: &str,
        fields: &ServerFields,
        clear_trust_and_allowlist: bool,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let update = Statement::new(UPDATE_SERVER)
            .bind(id)
            .bind(fields.name.as_str())
            .bind(fields.host.as_str())
            .bind(i64::from(fields.port))
            .bind(fields.username.as_str())
            .bind(fields.note.as_str())
            .bind(now.as_millis());
        let mut batch = WriteBatch::new()
            .push(update, Expect::Changes(1))
            .on_constraint(name_constraint(), NAME_TAKEN);

        if clear_trust_and_allowlist {
            batch = batch
                .push(
                    Statement::new(CLEAR_PINNED_CERT)
                        .bind(id)
                        .bind(now.as_millis()),
                    Expect::Changes(1),
                )
                .push(
                    Statement::new(DELETE_ALLOWED_VMS_FOR_SERVER).bind(id),
                    Expect::Any,
                );
        }

        self.write(batch, clear_trust_and_allowlist).await
    }

    /// Pins a verified leaf SHA-256 certificate digest and subject on a server.
    pub(crate) async fn set_pinned_cert(
        &self,
        id: &str,
        sha256: &str,
        subject: &str,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let update = Statement::new(SET_PINNED_CERT)
            .bind(id)
            .bind(sha256)
            .bind(subject)
            .bind(now.as_millis());
        self.write(WriteBatch::new().push(update, Expect::Changes(1)), false)
            .await
    }

    /// Clears a server's pinned leaf certificate, returning it to OS trust.
    pub(crate) async fn clear_pinned_cert(
        &self,
        id: &str,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let update = Statement::new(CLEAR_PINNED_CERT)
            .bind(id)
            .bind(now.as_millis());
        self.write(WriteBatch::new().push(update, Expect::Changes(1)), false)
            .await
    }

    /// Removes a server and cascades deletion of its allowed VMs.
    pub(crate) async fn delete_server(&self, id: &str) -> Result<(), StoreError> {
        let delete = Statement::new(DELETE_SERVER).bind(id);
        self.write(WriteBatch::new().push(delete, Expect::Changes(1)), true)
            .await
    }

    /// Replaces the server's explicit allowed VM list atomically.
    pub(crate) async fn replace_allowed_vms(
        &self,
        server_id: &str,
        vms: &[AllowedVmEntry],
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let mut batch = WriteBatch::new().push(
            Statement::new(DELETE_ALLOWED_VMS_FOR_SERVER).bind(server_id),
            Expect::Any,
        );

        // Deduplicate by VMID defensively before inserting.
        let mut seen = std::collections::BTreeSet::new();
        for vm in vms {
            if !seen.insert(vm.vmid) {
                continue;
            }

            let insert = Statement::new(INSERT_ALLOWED_VM)
                .bind(server_id)
                .bind(i64::from(vm.vmid))
                .bind(vm.node.as_str())
                .bind(vm.name.trim())
                .bind(now.as_millis());
            batch = batch.push(insert, Expect::Changes(1));
        }

        self.write(batch, true).await
    }

    /// Refreshes cached node and name metadata for an already-allowed VM.
    pub(crate) async fn update_allowed_vm_metadata(
        &self,
        server_id: &str,
        vmid: u32,
        node: &str,
        name: &str,
    ) -> Result<(), StoreError> {
        let update = Statement::new(UPDATE_ALLOWED_VM_METADATA)
            .bind(server_id)
            .bind(i64::from(vmid))
            .bind(node)
            .bind(name);
        self.write(WriteBatch::new().push(update, Expect::Any), false)
            .await
    }

    async fn write(&self, batch: WriteBatch, erase: bool) -> Result<(), StoreError> {
        let durability = if erase {
            Durability::Erase
        } else {
            Durability::Full
        };
        self.store.write(batch, durability).await.map(drop)
    }
}

fn name_constraint() -> Constraint {
    Constraint::Key {
        table: "proxmox_servers",
        columns: &["name"],
    }
}

fn parse_server(row: &Row) -> Result<ServerRecord, StoreError> {
    let port_raw: i64 = row.get(3)?;
    let port = u16::try_from(port_raw).map_err(|_err| StoreError::InvalidValue { column: 3 })?;

    Ok(ServerRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        port,
        username: row.get(4)?,
        password_set: row.get(5)?,
        pinned_cert_sha256: row.get(6)?,
        pinned_cert_subject: row.get(7)?,
        pinned_cert_at: row.get(8)?,
        note: row.get(9)?,
    })
}

fn parse_allowed_vm(row: &Row) -> Result<AllowedVmRecord, StoreError> {
    let vmid_raw: i64 = row.get(1)?;
    let vmid = u32::try_from(vmid_raw).map_err(|_err| StoreError::InvalidValue { column: 1 })?;

    Ok(AllowedVmRecord {
        server_id: row.get(0)?,
        vmid,
        node: row.get(2)?,
        name: row.get(3)?,
        added_at: row.get(4)?,
    })
}
