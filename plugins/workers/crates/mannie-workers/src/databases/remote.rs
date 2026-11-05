//! The master's calls of the link's package `databases` (ADR-0057): the
//! databases on a worker's host, which the worker runs under its ceiling.
//! A worker that is not connected, or whose version lacks the package,
//! gets no call; a refusal comes back as the worker's document; and a call
//! that failed on the way says so, which a write takes as uncertain, since
//! the worker may have run it.

use mannie_link::codec::{Answer, MAX_MESSAGE_BYTES, answer, from_json, to_json};
use mannie_link::databases::{
    DatabaseRefusal, HostDatabase, RegisterDatabase, UpdateDatabase, WriteDone,
};
use mannie_link::levels;
use mannie_link::proto::databases::databases_service_client::DatabasesServiceClient;
use mannie_link::proto::databases::{
    BrowseRequest, CountRequest, ForgetRequest, ListRequest, ReadRequest, RegisterRequest,
    StructureRequest, UpdateRequest, WriteRequest,
};
use mannie_plugin_sdk::contracts::{DatabaseKey, WorkerId};
use mannie_plugin_sdk::services::sqlite_files::{
    DatabaseStructure, Page, PageRequest, ReadLimits, Statement, StatementRows, WriteEvidence,
};
use tonic::Status;
use tonic::transport::Channel;

use crate::hub::Hub;

/// Why a call of a worker's databases did not answer as asked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RemoteError {
    /// The worker is not connected (ADR-0057, decision 9).
    Offline,
    /// The worker's version does not have the link's package of databases.
    NeedsUpdate,
    /// The worker refused, with its document.
    Refused(DatabaseRefusal),
    /// The call failed on the way, or its answer was lost.
    Lost,
}

/// What a status from the worker means: its refusal, or a call that failed
/// on the way.
fn failed(status: &Status) -> RemoteError {
    match answer(status) {
        Answer::Refused(message) => match serde_json::from_str::<DatabaseRefusal>(&message) {
            Ok(refusal) => RemoteError::Refused(refusal),
            Err(_unreadable) => {
                tracing::warn!("a worker refused a request on a database without a document");
                RemoteError::Lost
            }
        },
        Answer::Unreachable => RemoteError::Lost,
    }
}

/// A document of the worker's answer, or a lost call for one that does not
/// read, since the worker's version wrote what this one does not know.
fn document<T: serde::de::DeserializeOwned>(text: &str, what: &str) -> Result<T, RemoteError> {
    from_json(text, what).map_err(|_unreadable| RemoteError::Lost)
}

/// The JSON of a document for a request.
fn json<T: serde::Serialize>(value: &T) -> Result<String, RemoteError> {
    to_json(value).map_err(|_unwritable| RemoteError::Lost)
}

/// The client of one worker's databases.
#[derive(Debug, Clone)]
pub(crate) struct Remote {
    client: DatabasesServiceClient<Channel>,
}

impl Remote {
    /// The client over the worker's connection, while the worker is
    /// connected and speaks the package.
    pub(crate) fn of(hub: &Hub, worker: &WorkerId) -> Result<Self, RemoteError> {
        let connection = hub.connection(worker).ok_or(RemoteError::Offline)?;
        if !levels::speaks(&connection.levels, levels::DATABASES) {
            return Err(RemoteError::NeedsUpdate);
        }
        Ok(Self {
            client: DatabasesServiceClient::new(connection.channel.clone())
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
        })
    }

    /// Registers the file at a path that the owner typed.
    pub(crate) async fn register(
        mut self,
        register: &RegisterDatabase,
    ) -> Result<HostDatabase, RemoteError> {
        let response = self
            .client
            .register(RegisterRequest {
                register_json: json(register)?,
            })
            .await
            .map_err(|status| failed(&status))?
            .into_inner();
        document(&response.database_json, "the registered database")
    }

    /// The worker's databases.
    pub(crate) async fn list(mut self) -> Result<Vec<HostDatabase>, RemoteError> {
        let response = self
            .client
            .list(ListRequest {})
            .await
            .map_err(|status| failed(&status))?
            .into_inner();
        document(&response.databases_json, "the worker's databases")
    }

    /// Renames a database, marks it, or accepts its file.
    pub(crate) async fn update(
        mut self,
        key: &DatabaseKey,
        update: &UpdateDatabase,
    ) -> Result<HostDatabase, RemoteError> {
        let response = self
            .client
            .update(UpdateRequest {
                key: key.to_string(),
                update_json: json(update)?,
            })
            .await
            .map_err(|status| failed(&status))?
            .into_inner();
        document(&response.database_json, "the changed database")
    }

    /// Forgets a database.
    pub(crate) async fn forget(mut self, key: &DatabaseKey) -> Result<(), RemoteError> {
        self.client
            .forget(ForgetRequest {
                key: key.to_string(),
            })
            .await
            .map_err(|status| failed(&status))?;
        Ok(())
    }

    /// A database's structure.
    pub(crate) async fn structure(
        mut self,
        key: &DatabaseKey,
    ) -> Result<DatabaseStructure, RemoteError> {
        let response = self
            .client
            .structure(StructureRequest {
                key: key.to_string(),
            })
            .await
            .map_err(|status| failed(&status))?
            .into_inner();
        document(&response.structure_json, "the database's structure")
    }

    /// Counts the rows of a table or a view.
    pub(crate) async fn count(
        mut self,
        key: &DatabaseKey,
        table: String,
    ) -> Result<u64, RemoteError> {
        let response = self
            .client
            .count(CountRequest {
                key: key.to_string(),
                table,
            })
            .await
            .map_err(|status| failed(&status))?
            .into_inner();
        Ok(response.rows)
    }

    /// One page of a table.
    pub(crate) async fn page(
        mut self,
        key: &DatabaseKey,
        request: &PageRequest,
    ) -> Result<Page, RemoteError> {
        let response = self
            .client
            .browse(BrowseRequest {
                key: key.to_string(),
                request_json: json(request)?,
            })
            .await
            .map_err(|status| failed(&status))?
            .into_inner();
        document(&response.page_json, "the page")
    }

    /// Runs one statement that changes nothing. Dropping the future cancels
    /// the call, and the worker stops the statement.
    pub(crate) async fn read(
        mut self,
        key: &DatabaseKey,
        statement: &Statement,
        limits: ReadLimits,
    ) -> Result<StatementRows, RemoteError> {
        let response = self
            .client
            .read(ReadRequest {
                key: key.to_string(),
                statement_json: json(statement)?,
                limits_json: json(&limits)?,
            })
            .await
            .map_err(|status| failed(&status))?
            .into_inner();
        document(&response.rows_json, "the rows")
    }

    /// Runs a write with its evidence, once for its operation ID.
    pub(crate) async fn write(
        mut self,
        evidence: &WriteEvidence,
    ) -> Result<WriteDone, RemoteError> {
        let response = self
            .client
            .write(WriteRequest {
                evidence_json: json(evidence)?,
            })
            .await
            .map_err(|status| failed(&status))?
            .into_inner();
        document(&response.done_json, "what the write did")
    }
}
