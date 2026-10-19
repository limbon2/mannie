//! The executor's `Delegation` port over the link (ADR-0027, decision 1):
//! each call goes to the worker's services over its working connection. A
//! worker that has no connection now, or a call that fails on the way, is
//! unreachable; a worker's refusal comes back as its error.

use futures_util::StreamExt;
use futures_util::future::{BoxFuture, FutureExt};
use futures_util::stream::BoxStream;
use mannie_link::codec::MAX_MESSAGE_BYTES;
use mannie_link::codec::{Answer, answer, from_json, to_json};
use mannie_link::proto::approvals::DecideRequest;
use mannie_link::proto::approvals::approvals_service_client::ApprovalsServiceClient;
use mannie_link::proto::events::events_service_client::EventsServiceClient;
use mannie_link::proto::events::{
    CheckpointRequest, EarlierTurnsRequest, SubscribeRequest, ToolOutputRequest,
};
use mannie_link::proto::leases::leases_service_client::LeasesServiceClient;
use mannie_link::proto::leases::{DeliverRequest, RevokeRequest};
use mannie_link::proto::session::UnpairRequest;
use mannie_link::proto::session::session_service_client::SessionServiceClient;
use mannie_link::proto::tasks::tasks_service_client::TasksServiceClient;
use mannie_link::proto::tasks::{
    CancelRequest, DeleteConversationsRequest, GetRequest, SubmitRequest,
};
use mannie_plugin_sdk::contracts::{
    AssignedTaskState, ChatError, Checkpoint, CheckpointRef, ConversationId, ConversationUpdate,
    DecideApproval, EarlierTurns, LoadEarlierTurns, ReadToolOutput, Secret, TaskAssignment, TaskId,
    ToolOutputPage, ToolsError, WorkerId,
};
use tokio::sync::watch;
use tonic::Status;
use tonic::transport::Channel;

use super::Hub;
use super::connection::Connection;
use crate::delegation::{Delegation, DelegationError};
use crate::view::WorkerView;

/// How long removing a worker waits for it to unpair.
const UNPAIR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn delegation_error(status: &Status) -> DelegationError {
    match answer(status) {
        Answer::Refused(code) => DelegationError::Refused { code },
        Answer::Unreachable => DelegationError::Unreachable,
    }
}

fn chat_error(status: &Status) -> ChatError {
    match answer(status) {
        Answer::Refused(json) => serde_json::from_str(&json).unwrap_or_else(|_unknown| {
            tracing::warn!("a worker refused a read with an error that this version does not know");
            ChatError::StorageFailed
        }),
        Answer::Unreachable => ChatError::WorkerUnreachable,
    }
}

fn tools_error(status: &Status) -> ToolsError {
    match answer(status) {
        Answer::Refused(json) => serde_json::from_str(&json).unwrap_or_else(|_unknown| {
            tracing::warn!("a worker refused a call with an error that this version does not know");
            ToolsError::StorageFailed
        }),
        Answer::Unreachable => ToolsError::WorkerUnreachable,
    }
}

fn decoded<T: serde::de::DeserializeOwned>(json: &str, what: &str) -> Result<T, DelegationError> {
    from_json(json, what).map_err(|_invalid| DelegationError::Unreachable)
}

fn encoded<T: serde::Serialize>(value: &T) -> Result<String, DelegationError> {
    to_json(value).map_err(|_unwritable| DelegationError::Unreachable)
}

fn tasks(channel: Channel) -> TasksServiceClient<Channel> {
    TasksServiceClient::new(channel)
        .max_decoding_message_size(MAX_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_MESSAGE_BYTES)
}

fn events(channel: Channel) -> EventsServiceClient<Channel> {
    EventsServiceClient::new(channel)
        .max_decoding_message_size(MAX_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_MESSAGE_BYTES)
}

/// Asks a worker to end the pairing, before it is removed.
pub(super) async fn unpair(connection: &Connection) -> Result<(), DelegationError> {
    let mut client = SessionServiceClient::new(connection.channel.clone());
    let call = client.unpair(UnpairRequest {});
    match tokio::time::timeout(UNPAIR_TIMEOUT, call).await {
        Ok(Ok(_response)) => Ok(()),
        Ok(Err(status)) => Err(delegation_error(&status)),
        Err(_elapsed) => Err(DelegationError::Unreachable),
    }
}

impl Hub {
    fn channel_to(&self, worker: &WorkerId) -> Result<Channel, DelegationError> {
        self.connection(worker)
            .map(|connection| connection.channel.clone())
            .ok_or(DelegationError::Unreachable)
    }
}

impl Delegation for Hub {
    fn workers(&self) -> watch::Receiver<Vec<WorkerView>> {
        self.worker_views()
    }

    fn submit(
        &self,
        worker: &WorkerId,
        assignment: TaskAssignment,
    ) -> BoxFuture<'static, Result<AssignedTaskState, DelegationError>> {
        let channel = self.channel_to(worker);
        async move {
            let request = SubmitRequest {
                assignment_json: encoded(&assignment)?,
            };
            let response = tasks(channel?)
                .submit(request)
                .await
                .map_err(|status| delegation_error(&status))?
                .into_inner();
            decoded(&response.state_json, "a task's state")
        }
        .boxed()
    }

    fn task(
        &self,
        worker: &WorkerId,
        task: &TaskId,
    ) -> BoxFuture<'static, Result<Option<AssignedTaskState>, DelegationError>> {
        let channel = self.channel_to(worker);
        let task_id = task.as_str().to_owned();
        async move {
            let response = tasks(channel?)
                .get(GetRequest { task_id })
                .await
                .map_err(|status| delegation_error(&status))?
                .into_inner();
            response
                .state_json
                .as_deref()
                .map(|json| decoded(json, "a task's state"))
                .transpose()
        }
        .boxed()
    }

    fn cancel(
        &self,
        worker: &WorkerId,
        task: &TaskId,
    ) -> BoxFuture<'static, Result<Option<AssignedTaskState>, DelegationError>> {
        let channel = self.channel_to(worker);
        let task_id = task.as_str().to_owned();
        async move {
            let response = tasks(channel?)
                .cancel(CancelRequest { task_id })
                .await
                .map_err(|status| delegation_error(&status))?
                .into_inner();
            response
                .state_json
                .as_deref()
                .map(|json| decoded(json, "a task's state"))
                .transpose()
        }
        .boxed()
    }

    fn watch(
        &self,
        worker: &WorkerId,
    ) -> BoxFuture<'static, Result<BoxStream<'static, AssignedTaskState>, DelegationError>> {
        let watched = self
            .connection(worker)
            .map(|connection| connection.watch())
            .ok_or(DelegationError::Unreachable);
        async move { watched }.boxed()
    }

    fn deliver_lease(
        &self,
        worker: &WorkerId,
        task: &TaskId,
        binding: String,
        credential: Secret,
    ) -> BoxFuture<'static, Result<(), DelegationError>> {
        let channel = self.channel_to(worker);
        let task_id = task.as_str().to_owned();
        async move {
            let request = DeliverRequest {
                task_id,
                binding,
                credential: credential.expose().as_bytes().to_vec(),
            };
            LeasesServiceClient::new(channel?)
                .deliver(request)
                .await
                .map_err(|status| delegation_error(&status))?;
            Ok(())
        }
        .boxed()
    }

    fn revoke_lease(
        &self,
        worker: &WorkerId,
        task: &TaskId,
    ) -> BoxFuture<'static, Result<(), DelegationError>> {
        let channel = self.channel_to(worker);
        let task_id = task.as_str().to_owned();
        async move {
            LeasesServiceClient::new(channel?)
                .revoke(RevokeRequest { task_id })
                .await
                .map_err(|status| delegation_error(&status))?;
            Ok(())
        }
        .boxed()
    }

    fn delete_conversations(
        &self,
        worker: &WorkerId,
        conversations: Vec<ConversationId>,
    ) -> BoxFuture<'static, Result<(), DelegationError>> {
        let channel = self.channel_to(worker);
        async move {
            let request = DeleteConversationsRequest {
                conversation_ids: conversations
                    .iter()
                    .map(|conversation| conversation.as_str().to_owned())
                    .collect(),
            };
            tasks(channel?)
                .delete_conversations(request)
                .await
                .map_err(|status| delegation_error(&status))?;
            Ok(())
        }
        .boxed()
    }

    fn subscribe(
        &self,
        worker: &WorkerId,
        conversation: &ConversationId,
        after_seq: Option<u32>,
    ) -> BoxFuture<'static, Result<BoxStream<'static, ConversationUpdate>, ChatError>> {
        let channel = self.channel_to(worker);
        let conversation_id = conversation.as_str().to_owned();
        async move {
            let channel = channel.map_err(|_unreachable| ChatError::WorkerUnreachable)?;
            let streaming = events(channel)
                .subscribe(SubscribeRequest {
                    conversation_id,
                    after_seq,
                })
                .await
                .map_err(|status| chat_error(&status))?
                .into_inner();
            // An update that does not parse or a failure on the way ends the
            // stream: the subscriber follows again from its cursor.
            let updates = futures_util::stream::unfold(streaming, |mut streaming| async move {
                let message = streaming.message().await.ok().flatten()?;
                let update =
                    from_json::<ConversationUpdate>(&message.update_json, "an update").ok()?;
                Some((update, streaming))
            });
            Ok(updates.boxed())
        }
        .boxed()
    }

    fn earlier_turns(
        &self,
        worker: &WorkerId,
        input: LoadEarlierTurns,
    ) -> BoxFuture<'static, Result<EarlierTurns, ChatError>> {
        let channel = self.channel_to(worker);
        async move {
            let channel = channel.map_err(|_unreachable| ChatError::WorkerUnreachable)?;
            let input_json = to_json(&input).map_err(|_unwritable| ChatError::StorageFailed)?;
            let response = events(channel)
                .earlier_turns(EarlierTurnsRequest { input_json })
                .await
                .map_err(|status| chat_error(&status))?
                .into_inner();
            from_json(&response.turns_json, "earlier turns")
                .map_err(|_invalid| ChatError::WorkerUnreachable)
        }
        .boxed()
    }

    fn checkpoint(
        &self,
        worker: &WorkerId,
        input: CheckpointRef,
    ) -> BoxFuture<'static, Result<Checkpoint, ChatError>> {
        let channel = self.channel_to(worker);
        async move {
            let channel = channel.map_err(|_unreachable| ChatError::WorkerUnreachable)?;
            let input_json = to_json(&input).map_err(|_unwritable| ChatError::StorageFailed)?;
            let response = events(channel)
                .checkpoint(CheckpointRequest { input_json })
                .await
                .map_err(|status| chat_error(&status))?
                .into_inner();
            from_json(&response.checkpoint_json, "a checkpoint")
                .map_err(|_invalid| ChatError::WorkerUnreachable)
        }
        .boxed()
    }

    fn tool_output(
        &self,
        worker: &WorkerId,
        input: ReadToolOutput,
    ) -> BoxFuture<'static, Result<ToolOutputPage, ToolsError>> {
        let channel = self.channel_to(worker);
        async move {
            let channel = channel.map_err(|_unreachable| ToolsError::WorkerUnreachable)?;
            let input_json = to_json(&input).map_err(|_unwritable| ToolsError::StorageFailed)?;
            let response = events(channel)
                .tool_output(ToolOutputRequest { input_json })
                .await
                .map_err(|status| tools_error(&status))?
                .into_inner();
            from_json(&response.page_json, "a page of an output")
                .map_err(|_invalid| ToolsError::WorkerUnreachable)
        }
        .boxed()
    }

    fn decide_approval(
        &self,
        worker: &WorkerId,
        input: DecideApproval,
    ) -> BoxFuture<'static, Result<(), ToolsError>> {
        let channel = self.channel_to(worker);
        async move {
            let channel = channel.map_err(|_unreachable| ToolsError::WorkerUnreachable)?;
            let decision_json = to_json(&input).map_err(|_unwritable| ToolsError::StorageFailed)?;
            ApprovalsServiceClient::new(channel)
                .decide(DecideRequest {
                    decision_json,
                    by_full_access: false,
                })
                .await
                .map_err(|status| tools_error(&status))?;
            Ok(())
        }
        .boxed()
    }

    fn allow_by_full_access(
        &self,
        worker: &WorkerId,
        input: DecideApproval,
    ) -> BoxFuture<'static, Result<(), ToolsError>> {
        let channel = self.channel_to(worker);
        async move {
            let channel = channel.map_err(|_unreachable| ToolsError::WorkerUnreachable)?;
            let decision_json = to_json(&input).map_err(|_unwritable| ToolsError::StorageFailed)?;
            ApprovalsServiceClient::new(channel)
                .decide(DecideRequest {
                    decision_json,
                    by_full_access: true,
                })
                .await
                .map_err(|status| tools_error(&status))?;
            Ok(())
        }
        .boxed()
    }
}
