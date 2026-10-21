//! The event hubs (ADR-0012): every change to a conversation goes through its
//! hub, which stores the change and then publishes its events while it holds
//! the conversation's lock, so subscribers receive events in the order of
//! their sequence numbers.
//!
//! A subscription registers before it reads what is stored, and then skips
//! live events that the read already covered. An event published before the
//! registration was committed before the read began, so the read sees it; an
//! event published after it is in the subscription's queue. Nothing falls in
//! between, and nothing arrives twice.
//!
//! The conversation of a task on a worker lives on the worker; its
//! subscription forwards the worker's updates and puts the master's summary
//! of the conversation in place of the worker's (ADR-0027, decision 4).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use mannie_store::{Applied, Change, Durability, EventsAfter, RuntimeStoreError, Store};
use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::projects::ProjectView;
use mannie_types::transcript::{
    ConversationEvent, ConversationListUpdate, ConversationSummary, ConversationUpdate,
    SubscriptionEnd,
};
use tokio::sync::mpsc;

/// What a conversation's hub sends its subscribers. Every subscriber shares
/// one copy of an event until it takes the event out.
#[derive(Debug, Clone)]
enum HubMessage {
    Event(u32, Arc<ConversationEvent>),
    End(SubscriptionEnd),
}

/// What the list's hub sends its subscribers.
#[derive(Debug, Clone)]
enum ListMessage {
    Upserted(Box<ConversationSummary>),
    Removed(ConversationId),
    ProjectUpserted(ProjectView),
    ProjectRemoved(ProjectId),
    End(SubscriptionEnd),
}

#[derive(Debug, Default)]
struct ConversationHub {
    subscribers: tokio::sync::Mutex<Vec<mpsc::Sender<HubMessage>>>,
}

/// The hubs of every conversation and of the conversation list.
#[derive(Debug, Default)]
pub(crate) struct Hubs {
    conversations: Mutex<HashMap<ConversationId, Arc<ConversationHub>>>,
    list: tokio::sync::Mutex<Vec<mpsc::Sender<ListMessage>>>,
}

/// Sends a message to every subscriber, and drops the subscribers whose queue
/// is full or who went away. A dropped subscriber sees its queue close and
/// subscribes again.
fn publish<T: Clone>(subscribers: &mut Vec<mpsc::Sender<T>>, message: &T) {
    subscribers.retain(|subscriber| subscriber.try_send(message.clone()).is_ok());
}

impl Hubs {
    fn hub(&self, conversation: &ConversationId) -> Arc<ConversationHub> {
        let mut hubs = self
            .conversations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(hubs.entry(conversation.clone()).or_default())
    }

    /// Applies changes to a conversation and publishes their events. While
    /// one batch is stored and published, the conversation's other batches
    /// wait, which keeps publishing in the order of the sequence numbers.
    pub(crate) async fn apply(
        &self,
        store: &Store,
        conversation: &ConversationId,
        changes: Vec<Change>,
        durability: Durability,
    ) -> Result<Applied, RuntimeStoreError> {
        let hub = self.hub(conversation);
        let mut subscribers = hub.subscribers.lock().await;
        let applied = store.apply(conversation, changes, durability).await?;
        for (seq, event) in &applied.events {
            publish(
                &mut subscribers,
                &HubMessage::Event(*seq, Arc::new(event.clone())),
            );
        }
        if let Some(summary) = &applied.summary {
            // The list's lock is taken inside the conversation's, so the list
            // receives one conversation's changes in their order too.
            let mut list = self.list.lock().await;
            publish(&mut list, &ListMessage::Upserted(Box::new(summary.clone())));
        }
        drop(subscribers);
        Ok(applied)
    }

    /// Subscribes to a conversation: a snapshot, or the events after
    /// `after_seq`, and then the live events.
    pub(crate) async fn subscribe(
        &self,
        store: &Store,
        conversation: &ConversationId,
        after_seq: Option<u32>,
        limits: SubscriptionLimits,
    ) -> Result<Option<ConversationSubscription>, RuntimeStoreError> {
        let (sender, receiver) = mpsc::channel(limits.queue);
        {
            let hub = self.hub(conversation);
            hub.subscribers.lock().await.push(sender);
        }
        let mut initial = VecDeque::new();
        let mut through = None;
        if let Some(cursor) = after_seq {
            match store
                .events_after(conversation, cursor, limits.max_replay)
                .await?
            {
                EventsAfter::Events { events, last_seq } => {
                    initial.extend(
                        events
                            .into_iter()
                            .map(|(seq, event)| ConversationUpdate::Event { seq, event }),
                    );
                    through = Some(last_seq);
                }
                EventsAfter::NotFound => return Ok(None),
                EventsAfter::UseSnapshot => {}
            }
        }
        if through.is_none() {
            let Some(snapshot) = store
                .snapshot(conversation, limits.turns, limits.parts)
                .await?
            else {
                return Ok(None);
            };
            through = Some(snapshot.through_seq);
            initial.push_back(ConversationUpdate::Snapshot { snapshot });
        }
        Ok(Some(ConversationSubscription {
            source: Source::Local {
                initial,
                receiver,
                through: through.unwrap_or_default(),
            },
            ended: false,
        }))
    }

    /// Subscribes to the conversation list: the whole list, then each change.
    pub(crate) async fn subscribe_list(
        &self,
        store: &Store,
        limits: SubscriptionLimits,
    ) -> Result<ListSubscription, RuntimeStoreError> {
        let (sender, receiver) = mpsc::channel(limits.queue);
        // The list's lock stays held while the list is read, so no change
        // lands between the read and the registration.
        let mut list = self.list.lock().await;
        let read = store.conversation_list().await?;
        list.push(sender);
        drop(list);
        Ok(ListSubscription {
            initial: Some(ConversationListUpdate::Snapshot {
                conversations: read.conversations,
                projects: read.projects,
            }),
            receiver,
            ended: false,
        })
    }

    /// Tells the list's subscribers about a conversation that was written
    /// without a hub, such as a seeded one.
    #[cfg(feature = "validation")]
    pub(crate) async fn announce(&self, summary: ConversationSummary) {
        let mut list = self.list.lock().await;
        publish(&mut list, &ListMessage::Upserted(Box::new(summary)));
    }

    /// Holds the conversation list while a project changes: the caller
    /// stores the change and publishes it through the guard, so the list
    /// receives the changes of projects in their order.
    pub(crate) async fn project_changes(&self) -> ProjectChanges<'_> {
        ProjectChanges {
            list: self.list.lock().await,
        }
    }

    /// Ends the subscriptions of a deleted conversation and removes it from
    /// the list.
    pub(crate) async fn deleted(&self, conversation: &ConversationId) {
        self.end_subscriptions(conversation).await;
        let mut list = self.list.lock().await;
        publish(&mut list, &ListMessage::Removed(conversation.clone()));
    }

    /// Ends the subscriptions of a deleted task conversation, which the list
    /// never showed.
    pub(crate) async fn ended_task_conversation(&self, conversation: &ConversationId) {
        self.end_subscriptions(conversation).await;
    }

    async fn end_subscriptions(&self, conversation: &ConversationId) {
        let hub = self.hub(conversation);
        let mut subscribers = hub.subscribers.lock().await;
        publish(&mut subscribers, &HubMessage::End(SubscriptionEnd::Deleted));
        subscribers.clear();
        drop(subscribers);
        self.conversations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(conversation);
    }

    /// Ends every subscription, because Mannie quits.
    pub(crate) async fn shut_down(&self) {
        let hubs: Vec<Arc<ConversationHub>> = self
            .conversations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(Arc::clone)
            .collect();
        for hub in hubs {
            let mut subscribers = hub.subscribers.lock().await;
            publish(
                &mut subscribers,
                &HubMessage::End(SubscriptionEnd::Shutdown),
            );
            subscribers.clear();
        }
        let mut list = self.list.lock().await;
        publish(&mut list, &ListMessage::End(SubscriptionEnd::Shutdown));
        list.clear();
    }
}

/// The conversation list's subscribers, held while a project changes.
pub(crate) struct ProjectChanges<'a> {
    list: tokio::sync::MutexGuard<'a, Vec<mpsc::Sender<ListMessage>>>,
}

impl ProjectChanges<'_> {
    /// A project was added or renamed.
    pub(crate) fn upserted(&mut self, project: ProjectView) {
        publish(&mut self.list, &ListMessage::ProjectUpserted(project));
    }

    /// A project was removed, after its conversations.
    pub(crate) fn removed(&mut self, project: ProjectId) {
        publish(&mut self.list, &ListMessage::ProjectRemoved(project));
    }
}

/// How much a snapshot holds, how far a replay reaches, and how far a
/// subscriber may fall behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionLimits {
    /// The most turns in a snapshot.
    pub turns: u32,
    /// The most parts in a snapshot.
    pub parts: u32,
    /// The most events a subscriber may be behind before it gets a snapshot
    /// instead.
    pub max_replay: u32,
    /// How many updates may wait for a subscriber. A subscriber that falls
    /// further behind is ended and subscribes again from its cursor.
    pub queue: usize,
}

impl Default for SubscriptionLimits {
    fn default() -> Self {
        Self {
            turns: 50,
            parts: 400,
            max_replay: 5_000,
            queue: 1_024,
        }
    }
}

/// One subscriber's view of a conversation.
#[derive(Debug)]
pub struct ConversationSubscription {
    source: Source,
    ended: bool,
}

/// Where a subscription's updates come from.
enum Source {
    /// The conversation's hub on this machine.
    Local {
        initial: VecDeque<ConversationUpdate>,
        receiver: mpsc::Receiver<HubMessage>,
        /// The sequence number of the last event delivered or covered.
        through: u32,
    },
    /// The worker that keeps the conversation of a task, through the link.
    Remote {
        stream: BoxStream<'static, ConversationUpdate>,
        /// The master's summary of the conversation.
        summary: Box<ConversationSummary>,
    },
}

impl std::fmt::Debug for Source {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local { through, .. } => formatter
                .debug_struct("Local")
                .field("through", through)
                .finish_non_exhaustive(),
            Self::Remote { summary, .. } => formatter
                .debug_struct("Remote")
                .field("conversation", &summary.id)
                .finish_non_exhaustive(),
        }
    }
}

/// A worker's summary of a task conversation, with what the master knows of
/// it in place of what the worker says: the title, the parent, the number
/// and the worker. The worker says whether its run is active.
pub(crate) fn localize_summary(
    master: &ConversationSummary,
    worker: &ConversationSummary,
) -> ConversationSummary {
    ConversationSummary {
        id: master.id.clone(),
        title: master.title.clone(),
        title_source: master.title_source,
        profile_id: master.profile_id.clone(),
        has_active_run: worker.has_active_run,
        awaiting_approval: worker.awaiting_approval,
        full_access: master.full_access,
        project_id: master.project_id.clone(),
        workspace: master.workspace.clone(),
        task: master.task.clone(),
        created_at: master.created_at,
        updated_at: worker.updated_at.max(master.updated_at),
    }
}

/// An update from a worker, with the master's summary in it.
fn localize(master: &ConversationSummary, update: ConversationUpdate) -> ConversationUpdate {
    match update {
        ConversationUpdate::Snapshot { mut snapshot } => {
            snapshot.conversation = localize_summary(master, &snapshot.conversation);
            ConversationUpdate::Snapshot { snapshot }
        }
        ConversationUpdate::Event {
            seq,
            event: ConversationEvent::ConversationChanged { conversation },
        } => ConversationUpdate::Event {
            seq,
            event: ConversationEvent::ConversationChanged {
                conversation: localize_summary(master, &conversation),
            },
        },
        other => other,
    }
}

impl ConversationSubscription {
    /// A subscription to a task conversation on a worker: the worker's
    /// updates, with `summary`, the master's summary, in place of the
    /// worker's.
    pub(crate) fn remote(
        stream: BoxStream<'static, ConversationUpdate>,
        summary: ConversationSummary,
    ) -> Self {
        Self {
            source: Source::Remote {
                stream,
                summary: Box::new(summary),
            },
            ended: false,
        }
    }

    /// Whether updates of the initial replay are still waiting. Replayed
    /// events show the current state of what they name, so a subscriber's
    /// view is consistent again once the replay is applied. A worker's
    /// conversation replays on the worker.
    pub fn is_replaying(&self) -> bool {
        match &self.source {
            Source::Local { initial, .. } => !initial.is_empty(),
            Source::Remote { .. } => false,
        }
    }

    /// The next update. After `Ended`, there are no more.
    pub async fn next(&mut self) -> Option<ConversationUpdate> {
        match &mut self.source {
            Source::Local {
                initial,
                receiver,
                through,
            } => {
                if let Some(update) = initial.pop_front() {
                    return Some(update);
                }
                if self.ended {
                    return None;
                }
                loop {
                    match receiver.recv().await {
                        Some(HubMessage::Event(seq, event)) => {
                            // The initial read already covered this event.
                            if seq <= *through {
                                continue;
                            }
                            *through = seq;
                            return Some(ConversationUpdate::Event {
                                seq,
                                event: Arc::unwrap_or_clone(event),
                            });
                        }
                        Some(HubMessage::End(reason)) => {
                            self.ended = true;
                            return Some(ConversationUpdate::Ended { reason });
                        }
                        None => {
                            // The hub dropped this subscriber because it fell
                            // behind.
                            self.ended = true;
                            return Some(ConversationUpdate::Ended {
                                reason: SubscriptionEnd::Lagged,
                            });
                        }
                    }
                }
            }
            Source::Remote { stream, summary } => {
                if self.ended {
                    return None;
                }
                if let Some(update) = stream.next().await {
                    if matches!(update, ConversationUpdate::Ended { .. }) {
                        self.ended = true;
                    }
                    Some(localize(summary, update))
                } else {
                    // The link dropped: the subscriber follows again from its
                    // cursor, and learns then whether the worker can be
                    // reached.
                    self.ended = true;
                    Some(ConversationUpdate::Ended {
                        reason: SubscriptionEnd::Lagged,
                    })
                }
            }
        }
    }
}

/// One subscriber's view of the conversation list.
#[derive(Debug)]
pub struct ListSubscription {
    initial: Option<ConversationListUpdate>,
    receiver: mpsc::Receiver<ListMessage>,
    ended: bool,
}

impl ListSubscription {
    /// The next update. After `Ended`, there are no more.
    pub async fn next(&mut self) -> Option<ConversationListUpdate> {
        if let Some(update) = self.initial.take() {
            return Some(update);
        }
        if self.ended {
            return None;
        }
        let update = match self.receiver.recv().await {
            Some(ListMessage::Upserted(conversation)) => {
                ConversationListUpdate::Upserted { conversation }
            }
            Some(ListMessage::Removed(id)) => ConversationListUpdate::Removed { id },
            Some(ListMessage::ProjectUpserted(project)) => {
                ConversationListUpdate::ProjectUpserted { project }
            }
            Some(ListMessage::ProjectRemoved(id)) => ConversationListUpdate::ProjectRemoved { id },
            Some(ListMessage::End(reason)) => {
                self.ended = true;
                ConversationListUpdate::Ended { reason }
            }
            None => {
                self.ended = true;
                ConversationListUpdate::Ended {
                    reason: SubscriptionEnd::Lagged,
                }
            }
        };
        Some(update)
    }
}
