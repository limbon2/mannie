//! Commands of the ChatGPT account (ADR-0062; docs/features/chatgpt-plan.md):
//! the account on a channel, the sign-in with ChatGPT, signing out, the
//! catalog of the plan's models and the notice that Mannie uses the plan. The
//! runtime checks every input and keeps the tokens; the webview never sees
//! the sign-in's URL or a token.

use mannie_runtime::Runtime;
use mannie_types::ipc::{ChannelBatch, CommandError};
use mannie_types::provider::{
    ChatGptAccountUpdate, ChatGptError, ChatGptModel, SignOutReport, StartChatGptSignIn,
};
use mannie_types::transcript::{SubscriptionEnd, SubscriptionId};
use tauri::State;
use tauri::ipc::Channel;

use crate::links;
use crate::state::Core;

fn runtime(core: &Core) -> Result<&Runtime, ChatGptError> {
    core.runtime().ok_or(ChatGptError::StoreUnavailable)
}

/// Logs a refused or failed command by its code, and wraps the error for the
/// UI.
fn refused(command: &'static str) -> impl FnOnce(ChatGptError) -> CommandError<ChatGptError> {
    move |error| {
        tracing::warn!(
            command,
            error.code = error.code(),
            "a ChatGPT account command was refused"
        );
        CommandError::new(error)
    }
}

/// Opens a sign-in's page in the system browser, only on the owner's click
/// (ADR-0062, decision 5).
fn open_page(core: &Core, url: &str) -> Result<(), ChatGptError> {
    links::open(core, url).map_err(|error| {
        tracing::warn!(
            error.code = error.code(),
            "the ChatGPT sign-in's page did not open"
        );
        ChatGptError::BrowserUnavailable
    })
}

/// Follows the ChatGPT account: as it is first, then after each change.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subscribe_chatgpt_account(
    core: State<'_, Core>,
    updates: Channel<ChannelBatch<ChatGptAccountUpdate>>,
) -> Result<SubscriptionId, CommandError<ChatGptError>> {
    let page = core.subscriptions().page();
    let mut account = runtime(&core)
        .map_err(refused("subscribe_chatgpt_account"))?
        .subscribe_chatgpt_account();
    Ok(core
        .subscriptions()
        .start(page, updates, |sender| async move {
            loop {
                let current = account.borrow_and_update().clone();
                if sender
                    .send(ChatGptAccountUpdate::Account { account: current })
                    .await
                    .is_err()
                {
                    break;
                }
                if account.changed().await.is_err() {
                    // The runtime is gone: Mannie quits.
                    let _sent = sender
                        .send(ChatGptAccountUpdate::Ended {
                            reason: SubscriptionEnd::Shutdown,
                        })
                        .await;
                    break;
                }
            }
        }))
}

/// Starts a sign-in with ChatGPT and opens OpenAI's sign-in page in the
/// system browser. The account's channel shows how the sign-in ends.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_chatgpt_sign_in(
    core: State<'_, Core>,
    input: StartChatGptSignIn,
) -> Result<(), CommandError<ChatGptError>> {
    let runtime = runtime(&core).map_err(refused("start_chatgpt_sign_in"))?;
    let url = runtime
        .begin_chatgpt_sign_in(input)
        .await
        .map_err(refused("start_chatgpt_sign_in"))?;
    if let Err(error) = open_page(&core, url.as_str()) {
        // A sign-in whose page cannot open would only wait.
        let _canceled = runtime.cancel_chatgpt_sign_in();
        return Err(refused("start_chatgpt_sign_in")(error));
    }
    Ok(())
}

/// Opens the page of the sign-in that waits for the browser again.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn reopen_chatgpt_sign_in_page(
    core: State<'_, Core>,
) -> Result<(), CommandError<ChatGptError>> {
    let url = runtime(&core)
        .and_then(Runtime::chatgpt_sign_in_page)
        .map_err(refused("reopen_chatgpt_sign_in_page"))?;
    open_page(&core, url.as_str()).map_err(refused("reopen_chatgpt_sign_in_page"))
}

/// Cancels the sign-in that waits for the browser.
#[tauri::command]
#[specta::specta]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri hands command arguments over by value"
)]
pub(crate) fn cancel_chatgpt_sign_in(
    core: State<'_, Core>,
) -> Result<(), CommandError<ChatGptError>> {
    runtime(&core)
        .and_then(Runtime::cancel_chatgpt_sign_in)
        .map_err(refused("cancel_chatgpt_sign_in"))
}

/// Signs out of ChatGPT: OpenAI revokes the session, and Mannie forgets the
/// tokens.
#[tauri::command]
#[specta::specta]
pub(crate) async fn sign_out_chatgpt(
    core: State<'_, Core>,
) -> Result<SignOutReport, CommandError<ChatGptError>> {
    runtime(&core)
        .map_err(refused("sign_out_chatgpt"))?
        .sign_out_chatgpt()
        .await
        .map_err(refused("sign_out_chatgpt"))
}

/// The models that the ChatGPT plan offers the signed-in account.
#[tauri::command]
#[specta::specta]
pub(crate) async fn list_chatgpt_models(
    core: State<'_, Core>,
) -> Result<Vec<ChatGptModel>, CommandError<ChatGptError>> {
    runtime(&core)
        .map_err(refused("list_chatgpt_models"))?
        .chatgpt_models()
        .await
        .map_err(refused("list_chatgpt_models"))
}

/// Records that the owner saw the notice that Mannie uses their ChatGPT plan.
#[tauri::command]
#[specta::specta]
pub(crate) async fn acknowledge_chatgpt_notice(
    core: State<'_, Core>,
) -> Result<(), CommandError<ChatGptError>> {
    runtime(&core)
        .map_err(refused("acknowledge_chatgpt_notice"))?
        .acknowledge_chatgpt_notice()
        .await
        .map_err(refused("acknowledge_chatgpt_notice"))
}
