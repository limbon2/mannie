//! The worker's side of the link's package `databases` (ADR-0057): the
//! databases that the owner registered on the host, which the runtime runs
//! under the host's ceiling. A request that changes the worker's store or a
//! file, a registration, a change of one or a write, runs on a task of its
//! own to its end, as `Submit` does, so a call whose answer was lost has
//! still happened once. A read stops when the master cancels its call:
//! tonic drops the handler, and the engine interrupts the statement.

use std::path::PathBuf;

use mannie_host_kit::runtime::{HostDatabaseError, HostFile, HostWritten};
use mannie_plugin_sdk::contracts::DatabaseKey;
use mannie_plugin_sdk::services::sqlite_files::{
    PageRequest, ReadLimits, Statement, WriteEvidence,
};
use tonic::{Request, Response, Status};

use super::services::{Services, to_the_end};
use mannie_link::codec::{from_json, refused_with, to_json};
use mannie_link::databases::{
    DatabaseRefusal, HostDatabase, RegisterDatabase, UpdateDatabase, WriteDone,
};
use mannie_link::proto::databases::databases_service_server::DatabasesService;
use mannie_link::proto::databases::{
    BrowseRequest, BrowseResponse, CountRequest, CountResponse, ForgetRequest, ForgetResponse,
    ListRequest, ListResponse, ReadRequest, ReadResponse, RegisterRequest, RegisterResponse,
    StructureRequest, StructureResponse, UpdateRequest, UpdateResponse, WriteRequest,
    WriteResponse,
};

/// A database's key from a request.
fn key(text: &str) -> Result<DatabaseKey, Status> {
    DatabaseKey::parse(text)
        .map_err(|_invalid| Status::invalid_argument("the database's key is not valid"))
}

/// A database of the host as the link carries it.
fn host_database(file: HostFile) -> HostDatabase {
    HostDatabase {
        file: file.file,
        identity: file.identity,
    }
}

/// The worker's refusal of a request on one of its databases.
fn refusal(error: HostDatabaseError) -> Status {
    let refusal = match error {
        HostDatabaseError::Refused(refusal) => DatabaseRefusal::Refused { refusal },
        HostDatabaseError::NeedsOwner { digest } => DatabaseRefusal::NeedsOwner { digest },
        HostDatabaseError::Files(error) => DatabaseRefusal::Files { error },
    };
    refused_with(&refusal)
}

#[tonic::async_trait]
impl DatabasesService for Services {
    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        let register: RegisterDatabase =
            from_json(&request.into_inner().register_json, "the registration")?;
        let databases = self.databases.clone();
        let file = to_the_end(async move {
            databases
                .register(
                    PathBuf::from(register.path),
                    register.name,
                    register.read_only,
                )
                .await
        })
        .await?
        .map_err(refusal)?;
        Ok(Response::new(RegisterResponse {
            database_json: to_json(&host_database(file))?,
        }))
    }

    async fn list(&self, _request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let files = self.databases.files().await.map_err(refusal)?;
        let listed: Vec<HostDatabase> = files.into_iter().map(host_database).collect();
        Ok(Response::new(ListResponse {
            databases_json: to_json(&listed)?,
        }))
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        let request = request.into_inner();
        let key = key(&request.key)?;
        let update: UpdateDatabase = from_json(&request.update_json, "the change")?;
        let databases = self.databases.clone();
        let file = to_the_end(async move {
            match update {
                UpdateDatabase::Rename { name } => databases.rename(key, name).await,
                UpdateDatabase::Mark { read_only } => databases.mark(key, read_only).await,
                UpdateDatabase::Accept => databases.accept(key).await,
            }
        })
        .await?
        .map_err(refusal)?;
        Ok(Response::new(UpdateResponse {
            database_json: to_json(&host_database(file))?,
        }))
    }

    async fn forget(
        &self,
        request: Request<ForgetRequest>,
    ) -> Result<Response<ForgetResponse>, Status> {
        let key = key(&request.into_inner().key)?;
        let databases = self.databases.clone();
        to_the_end(async move { databases.forget(key).await })
            .await?
            .map_err(refusal)?;
        Ok(Response::new(ForgetResponse {}))
    }

    async fn structure(
        &self,
        request: Request<StructureRequest>,
    ) -> Result<Response<StructureResponse>, Status> {
        let key = key(&request.into_inner().key)?;
        let structure = self.databases.structure(key).await.map_err(refusal)?;
        Ok(Response::new(StructureResponse {
            structure_json: to_json(&structure)?,
        }))
    }

    async fn count(
        &self,
        request: Request<CountRequest>,
    ) -> Result<Response<CountResponse>, Status> {
        let request = request.into_inner();
        let key = key(&request.key)?;
        let rows = self
            .databases
            .count(key, request.table)
            .await
            .map_err(refusal)?;
        Ok(Response::new(CountResponse { rows }))
    }

    async fn browse(
        &self,
        request: Request<BrowseRequest>,
    ) -> Result<Response<BrowseResponse>, Status> {
        let request = request.into_inner();
        let key = key(&request.key)?;
        let page_request: PageRequest = from_json(&request.request_json, "the page's request")?;
        let page = self
            .databases
            .page(key, page_request)
            .await
            .map_err(refusal)?;
        Ok(Response::new(BrowseResponse {
            page_json: to_json(&page)?,
        }))
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<Response<ReadResponse>, Status> {
        let request = request.into_inner();
        let key = key(&request.key)?;
        let statement: Statement = from_json(&request.statement_json, "the statement")?;
        let limits: ReadLimits = from_json(&request.limits_json, "the read's limits")?;
        let rows = self
            .databases
            .read(key, statement, limits)
            .await
            .map_err(refusal)?;
        Ok(Response::new(ReadResponse {
            rows_json: to_json(&rows)?,
        }))
    }

    async fn write(
        &self,
        request: Request<WriteRequest>,
    ) -> Result<Response<WriteResponse>, Status> {
        let evidence: WriteEvidence =
            from_json(&request.into_inner().evidence_json, "the write's evidence")?;
        let databases = self.databases.clone();
        let written = to_the_end(async move { databases.write(evidence).await })
            .await?
            .map_err(refusal)?;
        let done = match written {
            HostWritten::Statements(outcomes) => WriteDone::Statements { outcomes },
            HostWritten::Change(outcome) => WriteDone::Change { outcome },
        };
        Ok(Response::new(WriteResponse {
            done_json: to_json(&done)?,
        }))
    }
}
