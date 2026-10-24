//! Commands of the chat feature (docs/features/chat/conversations.md,
//! contracts). The runtime checks every request; these commands find the
//! runtime, log refusals by their code, and forward subscriptions to the
//! webview's channels.

use mannie_runtime::Runtime;
use mannie_types::chat::{
    ChatError, ChatSettings, CheckpointRef, ConversationRef, EarlierTurns, LoadEarlierTurns,
    RenameConversation, SendMessage, SetConversationProfile, StartConversation,
    StartedConversation, StartedReply, StopOutcome, SubscribeConversation, Unsubscribe,
    UpdateChatSettings,
};
use mannie_types::context::ChangeAttachment;
use mannie_types::images::{ImageData, ReadImage};
use mannie_types::ipc::{ChannelBatch, CommandError};
use mannie_types::mentions::{
    ListMentionProviders, MentionProviders, MentionSuggestions, SuggestMentions,
};
use mannie_types::transcript::{
    Checkpoint, ConversationListUpdate, ConversationUpdate, SubscriptionId,
};
use tauri::State;
use tauri::ipc::Channel;

use crate::state::Core;

pub(super) fn runtime(core: &Core) -> Result<&Runtime, ChatError> {
    core.runtime().ok_or(ChatError::StoreUnavailable)
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
pub(super) fn refused(command: &'static str) -> impl FnOnce(ChatError) -> CommandError<ChatError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a chat command was refused"
        );
        CommandError::new(error)
    }
}

/// Follows the conversation list: the whole list first, then each change.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subscribe_conversations(
    core: State<'_, Core>,
    updates: Channel<ChannelBatch<ConversationListUpdate>>,
) -> Result<SubscriptionId, CommandError<ChatError>> {
    let page = core.subscriptions().page();
    let mut subscription = runtime(&core)
        .map_err(refused("subscribe_conversations"))?
        .subscribe_list()
        .await
        .map_err(refused("subscribe_conversations"))?;
    Ok(core
        .subscriptions()
        .start(page, updates, |sender| async move {
            while let Some(update) = subscription.next().await {
                let ended = matches!(update, ConversationListUpdate::Ended { .. });
                if sender.send(update).await.is_err() || ended {
                    break;
                }
            }
        }))
}

/// Follows a conversation from the cursor `after_seq`, or from a snapshot.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subscribe_conversation(
    core: State<'_, Core>,
    input: SubscribeConversation,
    updates: Channel<ChannelBatch<ConversationUpdate>>,
) -> Result<SubscriptionId, CommandError<ChatError>> {
    let page = core.subscriptions().page();
    let mut subscription = runtime(&core)
        .map_err(refused("subscribe_conversation"))?
        .subscribe(input)
        .await
        .map_err(refused("subscribe_conversation"))?;
    Ok(core
        .subscriptions()
        .start(page, updates, |sender| async move {
            while let Some(update) = subscription.next().await {
                let ended = matches!(update, ConversationUpdate::Ended { .. });
                if sender.send(update).await.is_err() || ended {
                    break;
                }
            }
        }))
}

/// Ends a subscription. Ending one that already ended changes nothing.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn unsubscribe(core: State<'_, Core>, input: Unsubscribe) {
    core.subscriptions().cancel(input.subscription_id);
}

/// The turns before a position, page by page.
#[tauri::command]
#[specta::specta]
pub(crate) async fn load_earlier_turns(
    core: State<'_, Core>,
    input: LoadEarlierTurns,
) -> Result<EarlierTurns, CommandError<ChatError>> {
    runtime(&core)?
        .earlier_turns(input)
        .await
        .map_err(refused("load_earlier_turns"))
}

/// Creates a conversation with the owner's first message and starts the
/// reply.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_conversation(
    core: State<'_, Core>,
    input: StartConversation,
) -> Result<StartedConversation, CommandError<ChatError>> {
    runtime(&core)?
        .start_conversation(input)
        .await
        .map_err(refused("start_conversation"))
}

/// Adds the owner's message to a conversation and starts the reply.
#[tauri::command]
#[specta::specta]
pub(crate) async fn send_message(
    core: State<'_, Core>,
    input: SendMessage,
) -> Result<StartedReply, CommandError<ChatError>> {
    runtime(&core)?
        .send_message(input)
        .await
        .map_err(refused("send_message"))
}

/// What the owner can mention after "@" in the message box (ADR-0032): a
/// group for each kind of mention that the place offers, or for the one
/// asked for.
#[tauri::command]
#[specta::specta]
pub(crate) async fn suggest_mentions(
    core: State<'_, Core>,
    input: SuggestMentions,
) -> Result<MentionSuggestions, CommandError<ChatError>> {
    runtime(&core)?
        .suggest_mentions(input)
        .await
        .map_err(refused("suggest_mentions"))
}

/// The kinds of mention that the place offers (ADR-0032), which the "@" list
/// asks one by one, so that a slow kind holds back no other.
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_mention_providers(
    core: State<'_, Core>,
    input: ListMentionProviders,
) -> Result<MentionProviders, CommandError<ChatError>> {
    runtime(&core)?
        .list_mention_providers(input)
        .await
        .map_err(refused("list_mention_providers"))
}

/// Attaches a context provider's key, such as a notebook, to a conversation
/// (ADR-0046, decision 1), once the provider described it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn attach_context(
    core: State<'_, Core>,
    input: ChangeAttachment,
) -> Result<(), CommandError<ChatError>> {
    runtime(&core)?
        .attach_context(input)
        .await
        .map_err(refused("attach_context"))
}

/// Detaches a key from a conversation, also while its provider's plugin is
/// off.
#[tauri::command]
#[specta::specta]
pub(crate) async fn detach_context(
    core: State<'_, Core>,
    input: ChangeAttachment,
) -> Result<(), CommandError<ChatError>> {
    runtime(&core)?
        .detach_context(input)
        .await
        .map_err(refused("detach_context"))
}

/// The thumbnail or the full image of an image part (ADR-0033, decision 4),
/// also while the plugin that added it is off.
#[tauri::command]
#[specta::specta]
pub(crate) async fn read_image(
    core: State<'_, Core>,
    input: ReadImage,
) -> Result<ImageData, CommandError<ChatError>> {
    runtime(&core)?
        .read_image(input)
        .await
        .map_err(refused("read_image"))
}

/// Stops the conversation's reply and waits until it has ended.
#[tauri::command]
#[specta::specta]
pub(crate) async fn stop_reply(
    core: State<'_, Core>,
    input: ConversationRef,
) -> Result<StopOutcome, CommandError<ChatError>> {
    Ok(runtime(&core)?.stop_reply(&input.conversation_id).await)
}

/// Starts a new reply for the latest turn when its reply failed, stopped or
/// was interrupted.
#[tauri::command]
#[specta::specta]
pub(crate) async fn retry_reply(
    core: State<'_, Core>,
    input: ConversationRef,
) -> Result<StartedReply, CommandError<ChatError>> {
    runtime(&core)?
        .retry_reply(&input.conversation_id)
        .await
        .map_err(refused("retry_reply"))
}

/// Renames a conversation.
#[tauri::command]
#[specta::specta]
pub(crate) async fn rename_conversation(
    core: State<'_, Core>,
    input: RenameConversation,
) -> Result<(), CommandError<ChatError>> {
    runtime(&core)?
        .rename_conversation(input)
        .await
        .map_err(refused("rename_conversation"))
}

/// Deletes a conversation, stopping its reply first.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delete_conversation(
    core: State<'_, Core>,
    input: ConversationRef,
) -> Result<(), CommandError<ChatError>> {
    runtime(&core)?
        .delete_conversation(&input.conversation_id)
        .await
        .map_err(refused("delete_conversation"))
}

/// Chooses the profile of a conversation's next replies, or the default.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_conversation_profile(
    core: State<'_, Core>,
    input: SetConversationProfile,
) -> Result<(), CommandError<ChatError>> {
    runtime(&core)?
        .set_conversation_profile(input)
        .await
        .map_err(refused("set_conversation_profile"))
}

/// Compacts a conversation now.
#[tauri::command]
#[specta::specta]
pub(crate) async fn compact_conversation(
    core: State<'_, Core>,
    input: ConversationRef,
) -> Result<(), CommandError<ChatError>> {
    runtime(&core)?
        .compact_conversation(&input.conversation_id)
        .await
        .map_err(refused("compact_conversation"))
}

/// A checkpoint with its summary.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_checkpoint(
    core: State<'_, Core>,
    input: CheckpointRef,
) -> Result<Checkpoint, CommandError<ChatError>> {
    runtime(&core)?
        .checkpoint(input)
        .await
        .map_err(refused("get_checkpoint"))
}

/// The owner's chat settings.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_chat_settings(
    core: State<'_, Core>,
) -> Result<ChatSettings, CommandError<ChatError>> {
    runtime(&core)?
        .chat_settings()
        .await
        .map_err(refused("get_chat_settings"))
}

/// Saves the owner's chat settings and returns what was stored.
#[tauri::command]
#[specta::specta]
pub(crate) async fn update_chat_settings(
    core: State<'_, Core>,
    update: UpdateChatSettings,
) -> Result<ChatSettings, CommandError<ChatError>> {
    let settings = runtime(&core)?
        .update_chat_settings(update)
        .await
        .map_err(refused("update_chat_settings"))?;
    tracing::info!(
        instructions_bytes = settings.instructions.as_str().len(),
        "changed the chat settings"
    );
    Ok(settings)
}
