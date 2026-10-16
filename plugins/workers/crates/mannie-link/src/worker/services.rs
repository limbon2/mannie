//! The worker's services over its runtime (ADR-0028): the session, the
//! tasks, the events of their conversations, approvals and leases.
//!
//! `Submit`, `Cancel`, `Deliver` and `Decide` run on tasks of their own:
//! tonic drops a handler when its connection closes, and these must run to
//! their end, so that a call whose answer was lost has still happened once
//! and calling it again finds what it did.

use std::future::Future;
use std::sync::Arc;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use mannie_runtime::Runtime;
use mannie_types::chat::{ChatError, CheckpointRef, LoadEarlierTurns, SubscribeConversation};
use mannie_types::ids::{ConversationId, TaskId};
use mannie_types::secret::Secret;
use mannie_types::tasks::{AssignedTaskState, TaskAssignment};
use mannie_types::tools::{DecideApproval, ReadToolOutput};
use tokio::sync::watch;
use tonic::{Request, Response, Status};
use zeroize::Zeroizing;

use super::{Binding, LeaseVault, WorkerLink};
use crate::codec::{from_json, refused, refused_with, to_json};
use crate::host::WorkerDetails;
use crate::levels;
use crate::proto::approvals::approvals_service_server::ApprovalsService;
use crate::proto::approvals::{DecideRequest, DecideResponse};
use crate::proto::events::events_service_server::EventsService;
use crate::proto::events::{
    CheckpointRequest, CheckpointResponse, EarlierTurnsRequest, EarlierTurnsResponse,
    SubscribeRequest, SubscribeResponse, ToolOutputRequest, ToolOutputResponse,
};
use crate::proto::leases::leases_service_server::LeasesService;
use crate::proto::leases::{DeliverRequest, DeliverResponse, RevokeRequest, RevokeResponse};
use crate::proto::session::session_service_server::SessionService;
use crate::proto::session::{HelloRequest, HelloResponse, UnpairRequest, UnpairResponse};
use crate::proto::tasks::tasks_service_server::TasksService;
use crate::proto::tasks::{
    CancelRequest, CancelResponse, DeleteConversationsRequest, DeleteConversationsResponse,
    GetRequest, GetResponse, SubmitRequest, SubmitResponse, WatchRequest, WatchResponse,
};

/// The services of a paired worker. Cloning is cheap; clones share
/// everything.
#[derive(Debug, Clone)]
pub(crate) struct Services {
    runtime: Runtime,
    name: Arc<str>,
    details: Arc<WorkerDetails>,
    version: Arc<str>,
    binding: Arc<Binding>,
    leases: Arc<LeaseVault>,
    /// Turns true when the link stops.
    stops: watch::Receiver<bool>,
}

/// Resolves when the link stops, which ends the streams that a master keeps
/// open.
async fn link_stops(mut stops: watch::Receiver<bool>) {
    // A closed channel means that the link stopped too.
    let _stopped = stops.wait_for(|stopped| *stopped).await;
}

/// Runs `work` on a task of its own, so it ends even when the connection
/// that asked for it closes.
async fn to_the_end<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> Result<T, Status> {
    tokio::spawn(work).await.map_err(|error| {
        tracing::error!(%error, "a call of the link ended unexpectedly");
        Status::internal("the call ended unexpectedly")
    })
}

fn task_id(text: &str) -> Result<TaskId, Status> {
    TaskId::parse(text).map_err(|_invalid| Status::invalid_argument("the task's ID is not valid"))
}

fn state_json(state: Option<&AssignedTaskState>) -> Result<Option<String>, Status> {
    state.map(to_json).transpose()
}

impl Services {
    pub(crate) fn new(
        link: WorkerLink,
        binding: Arc<Binding>,
        stops: watch::Receiver<bool>,
    ) -> Self {
        Self {
            runtime: link.runtime,
            name: Arc::from(link.name),
            details: Arc::new(link.details),
            version: Arc::from(link.version),
            binding,
            leases: link.leases,
            stops,
        }
    }
}

#[tonic::async_trait]
impl SessionService for Services {
    async fn hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloResponse>, Status> {
        let hello = request.into_inner();
        tracing::info!(master_version = %hello.version, "the master connected");
        Ok(Response::new(HelloResponse {
            version: self.version.to_string(),
            levels: levels::levels(),
            name: self.name.to_string(),
            details_json: to_json(&*self.details)?,
        }))
    }

    async fn unpair(
        &self,
        _request: Request<UnpairRequest>,
    ) -> Result<Response<UnpairResponse>, Status> {
        let runtime = self.runtime.clone();
        let leases = Arc::clone(&self.leases);
        let binding = Arc::clone(&self.binding);
        to_the_end(async move {
            runtime.release_assignments().await;
            leases.clear();
            if let Err(error) = binding.clear() {
                tracing::error!(%error, "could not forget the master's pin");
                return Err(Status::internal("the master's pin could not be forgotten"));
            }
            tracing::info!("the master ended the pairing");
            Ok(())
        })
        .await??;
        Ok(Response::new(UnpairResponse {}))
    }
}

#[tonic::async_trait]
impl TasksService for Services {
    type WatchStream = BoxStream<'static, Result<WatchResponse, Status>>;

    async fn submit(
        &self,
        request: Request<SubmitRequest>,
    ) -> Result<Response<SubmitResponse>, Status> {
        let assignment: TaskAssignment =
            from_json(&request.into_inner().assignment_json, "the assignment")?;
        let runtime = self.runtime.clone();
        let state = to_the_end(async move { runtime.assign_task(assignment).await })
            .await?
            .map_err(|error| refused(error.code()))?;
        Ok(Response::new(SubmitResponse {
            state_json: to_json(&state)?,
        }))
    }

    async fn get(&self, request: Request<GetRequest>) -> Result<Response<GetResponse>, Status> {
        let task = task_id(&request.into_inner().task_id)?;
        let state = self
            .runtime
            .assigned_task(&task)
            .await
            .map_err(|error| refused(error.code()))?;
        Ok(Response::new(GetResponse {
            state_json: state_json(state.as_ref())?,
        }))
    }

    async fn cancel(
        &self,
        request: Request<CancelRequest>,
    ) -> Result<Response<CancelResponse>, Status> {
        let task = task_id(&request.into_inner().task_id)?;
        let runtime = self.runtime.clone();
        let state = to_the_end(async move { runtime.cancel_assigned_task(&task).await })
            .await?
            .map_err(|error| refused(error.code()))?;
        Ok(Response::new(CancelResponse {
            state_json: state_json(state.as_ref())?,
        }))
    }

    async fn watch(
        &self,
        _request: Request<WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        let watch = self.runtime.watch_assigned_tasks();
        let states = futures_util::stream::unfold(watch, |mut watch| async move {
            let state = watch.next().await?;
            Some((
                to_json(&state).map(|state_json| WatchResponse { state_json }),
                watch,
            ))
        });
        let states = states.take_until(link_stops(self.stops.clone()));
        Ok(Response::new(states.boxed()))
    }

    async fn delete_conversations(
        &self,
        request: Request<DeleteConversationsRequest>,
    ) -> Result<Response<DeleteConversationsResponse>, Status> {
        let conversations = request
            .into_inner()
            .conversation_ids
            .iter()
            .map(|id| ConversationId::parse(id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_invalid| Status::invalid_argument("a conversation's ID is not valid"))?;
        let runtime = self.runtime.clone();
        to_the_end(async move { runtime.delete_assigned_conversations(&conversations).await })
            .await?
            .map_err(|error| refused(error.code()))?;
        Ok(Response::new(DeleteConversationsResponse {}))
    }
}

#[tonic::async_trait]
impl EventsService for Services {
    type SubscribeStream = BoxStream<'static, Result<SubscribeResponse, Status>>;

    async fn subscribe(
        &self,
        request: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let request = request.into_inner();
        let conversation = ConversationId::parse(&request.conversation_id)
            .map_err(|_invalid| Status::invalid_argument("the conversation's ID is not valid"))?;
        let subscription = self
            .runtime
            .subscribe(SubscribeConversation {
                conversation_id: conversation,
                after_seq: request.after_seq,
            })
            .await
            .map_err(|error: ChatError| refused_with(&error))?;
        let updates = futures_util::stream::unfold(subscription, |mut subscription| async move {
            let update = subscription.next().await?;
            Some((
                to_json(&update).map(|update_json| SubscribeResponse { update_json }),
                subscription,
            ))
        });
        let updates = updates.take_until(link_stops(self.stops.clone()));
        Ok(Response::new(updates.boxed()))
    }

    async fn earlier_turns(
        &self,
        request: Request<EarlierTurnsRequest>,
    ) -> Result<Response<EarlierTurnsResponse>, Status> {
        let input: LoadEarlierTurns = from_json(&request.into_inner().input_json, "the request")?;
        let turns = self
            .runtime
            .earlier_turns(input)
            .await
            .map_err(|error| refused_with(&error))?;
        Ok(Response::new(EarlierTurnsResponse {
            turns_json: to_json(&turns)?,
        }))
    }

    async fn checkpoint(
        &self,
        request: Request<CheckpointRequest>,
    ) -> Result<Response<CheckpointResponse>, Status> {
        let input: CheckpointRef = from_json(&request.into_inner().input_json, "the request")?;
        let checkpoint = self
            .runtime
            .checkpoint(input)
            .await
            .map_err(|error| refused_with(&error))?;
        Ok(Response::new(CheckpointResponse {
            checkpoint_json: to_json(&checkpoint)?,
        }))
    }

    async fn tool_output(
        &self,
        request: Request<ToolOutputRequest>,
    ) -> Result<Response<ToolOutputResponse>, Status> {
        let input: ReadToolOutput = from_json(&request.into_inner().input_json, "the request")?;
        let page = self
            .runtime
            .tool_output(input)
            .await
            .map_err(|error| refused_with(&error))?;
        Ok(Response::new(ToolOutputResponse {
            page_json: to_json(&page)?,
        }))
    }
}

#[tonic::async_trait]
impl ApprovalsService for Services {
    async fn decide(
        &self,
        request: Request<DecideRequest>,
    ) -> Result<Response<DecideResponse>, Status> {
        let request = request.into_inner();
        let decision: DecideApproval = from_json(&request.decision_json, "the decision")?;
        let runtime = self.runtime.clone();
        to_the_end(async move {
            if request.by_full_access {
                runtime.allow_by_full_access(&decision)
            } else {
                runtime.decide_approval(decision).await
            }
        })
        .await?
        .map_err(|error| refused_with(&error))?;
        Ok(Response::new(DecideResponse {}))
    }
}

#[tonic::async_trait]
impl LeasesService for Services {
    async fn deliver(
        &self,
        request: Request<DeliverRequest>,
    ) -> Result<Response<DeliverResponse>, Status> {
        let DeliverRequest {
            task_id: task,
            binding,
            credential,
        } = request.into_inner();
        let task = task_id(&task)?;
        // The bytes become the secret without a copy, and the request's
        // buffer is cleared if they are not text.
        let credential = match String::from_utf8(credential) {
            Ok(text) => Secret::new(text),
            Err(error) => {
                drop(Zeroizing::new(error.into_bytes()));
                return Err(Status::invalid_argument("the credential is not UTF-8"));
            }
        };
        let runtime = self.runtime.clone();
        to_the_end(async move { runtime.deliver_lease(&task, &binding, credential).await })
            .await?
            .map_err(|error| refused(error.code()))?;
        Ok(Response::new(DeliverResponse {}))
    }

    async fn revoke(
        &self,
        request: Request<RevokeRequest>,
    ) -> Result<Response<RevokeResponse>, Status> {
        let task = task_id(&request.into_inner().task_id)?;
        self.runtime
            .revoke_lease(&task)
            .await
            .map_err(|error| refused(error.code()))?;
        Ok(Response::new(RevokeResponse {}))
    }
}
