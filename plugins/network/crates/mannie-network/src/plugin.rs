//! The Network Client plugin's definition (ADR-0075): manifest, store module,
//! start/stop lifecycle, tool/section/mention contributions and IPC handlers.

use std::sync::Arc;

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::StartContext;
use mannie_plugin_sdk::contracts::WorkItem;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Handlers, Interface, PluginStream};
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::management::{Management, PluginManagement};
use mannie_plugin_sdk::points::mentions::{MentionProvider, Mentions};
use mannie_plugin_sdk::points::sections::{Section, SectionBody, Sections};
use mannie_plugin_sdk::points::tools::{AgentSelector, PluginTool, Tools};
use mannie_plugin_sdk::services::file_dialog::FileDialog;
use mannie_plugin_sdk::services::secrets::Secrets;
use mannie_plugin_sdk::services::store::{Store, StoreModule};
use tokio::sync::watch;

use crate::commands::{
    CancelOwnerRequestCommand, CancelRunInput, ClearCookiesCommand, CollectionInput, CollectionRef,
    ConfirmOpenApiImportCommand, CookieInput, CookieRef, CreateCollectionCommand,
    CreateFolderCommand, CreateRequest, CreateRequestCommand, DeleteCollectionCommand,
    DeleteCookieCommand, DeleteEnvironmentCommand, DeleteFolderCommand, DeleteRequestCommand,
    DuplicateRequestCommand, EnvironmentInput, EnvironmentRef, FolderCreateInput, FolderRef,
    FollowStateChannel, GetStateCommand, ManualSendInput, OpenApiConfirmInput, OpenApiTextInput,
    PickOpenApiFile, PickOpenApiFileCommand, PreviewOpenApiTextCommand, RenameFolder,
    RenameFolderCommand, RequestRef, SaveCookieCommand, SaveEnvironmentCommand,
    SaveGlobalVariables, SaveGlobalVariablesCommand, SendOwnerRequestCommand, SetActiveEnvironment,
    SetActiveEnvironmentCommand, SetCollectionEnabled, SetCollectionEnabledCommand,
    UpdateCollection, UpdateCollectionCommand, UpdateRequest, UpdateRequestCommand, interface,
};
use crate::http::build_http_client;
use crate::management::NetworkManagement;
use crate::mentions::NetworkMentions;
use crate::model::{NetworkError, NetworkStateSnapshot};
use crate::section::{NetworkCollectionsSection, SECTION_ID, SECTION_POSITION};
use crate::state::NetworkManager;
use crate::store::MODULE;
use crate::tools::{
    GetRequestTool, ListCollectionsTool, ListRequestsTool, SEND_AGENTS, SendRequestTool,
};

/// The plugin's ID.
pub const NETWORK: &str = "network";

/// Start failure code when SQLite tables could not be loaded.
pub const STORAGE_FAILED: &str = "storage_failed";

/// Start failure code when the OS credential store could not be read.
pub const CREDENTIAL_STORE_FAILED: &str = "credential_store_failed";

/// Start failure code when the TLS/HTTP client could not be constructed.
pub const HTTP_FAILED: &str = "http_client_failed";

/// Work item code reported while manual workbench requests are in flight.
pub const ACTIVE_REQUESTS: &str = "active_requests";

/// The Network Client plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct NetworkPlugin;

impl NetworkPlugin {
    /// Creates a new stopped instance of the Network Client plugin.
    pub fn new() -> Self {
        Self
    }
}

fn state_stream(
    manager: NetworkManager,
    revisions: watch::Receiver<u64>,
) -> PluginStream<NetworkStateSnapshot> {
    stream::unfold(
        (manager, revisions, true),
        |(manager, mut revisions, first)| async move {
            if !first && revisions.changed().await.is_err() {
                return None;
            }

            revisions.borrow_and_update();
            let snapshot = manager.snapshot();
            Some((snapshot, (manager, revisions, false)))
        },
    )
    .boxed()
}

fn handlers(manager: &NetworkManager) -> Handlers {
    let base = add_collection_and_request_handlers(Handlers::new(), manager);
    add_env_cookie_and_workbench_handlers(base, manager)
}

fn add_collection_and_request_handlers(handlers: Handlers, manager: &NetworkManager) -> Handlers {
    let m_get = manager.clone();
    let m_create_col = manager.clone();
    let m_update_col = manager.clone();
    let m_enable_col = manager.clone();
    let m_delete_col = manager.clone();
    let m_create_folder = manager.clone();
    let m_rename_folder = manager.clone();
    let m_delete_folder = manager.clone();
    let m_create_req = manager.clone();
    let m_update_req = manager.clone();
    let m_dup_req = manager.clone();
    let m_delete_req = manager.clone();

    handlers
        .command::<GetStateCommand, _, _>(move |()| {
            std::future::ready(Ok::<_, NetworkError>(m_get.snapshot()))
        })
        .command::<CreateCollectionCommand, _, _>(move |input: CollectionInput| {
            let mgr = m_create_col.clone();
            async move { mgr.create_collection(input).await }
        })
        .command::<UpdateCollectionCommand, _, _>(move |input: UpdateCollection| {
            let mgr = m_update_col.clone();
            async move {
                mgr.update_collection(&input.collection_id, input.input)
                    .await
            }
        })
        .command::<SetCollectionEnabledCommand, _, _>(move |input: SetCollectionEnabled| {
            let mgr = m_enable_col.clone();
            async move {
                mgr.set_collection_enabled(&input.collection_id, input.enabled)
                    .await
            }
        })
        .command::<DeleteCollectionCommand, _, _>(move |input: CollectionRef| {
            let mgr = m_delete_col.clone();
            async move { mgr.delete_collection(&input.collection_id).await }
        })
        .command::<CreateFolderCommand, _, _>(move |input: FolderCreateInput| {
            let mgr = m_create_folder.clone();
            async move { mgr.create_folder(input).await }
        })
        .command::<RenameFolderCommand, _, _>(move |input: RenameFolder| {
            let mgr = m_rename_folder.clone();
            async move { mgr.rename_folder(&input.folder_id, &input.name).await }
        })
        .command::<DeleteFolderCommand, _, _>(move |input: FolderRef| {
            let mgr = m_delete_folder.clone();
            async move { mgr.delete_folder(&input.folder_id).await }
        })
        .command::<CreateRequestCommand, _, _>(move |input: CreateRequest| {
            let mgr = m_create_req.clone();
            async move { mgr.create_request(&input.collection_id, input.input).await }
        })
        .command::<UpdateRequestCommand, _, _>(move |input: UpdateRequest| {
            let mgr = m_update_req.clone();
            async move { mgr.update_request(&input.request_id, input.input).await }
        })
        .command::<DuplicateRequestCommand, _, _>(move |input: RequestRef| {
            let mgr = m_dup_req.clone();
            async move { mgr.duplicate_request(&input.request_id).await }
        })
        .command::<DeleteRequestCommand, _, _>(move |input: RequestRef| {
            let mgr = m_delete_req.clone();
            async move { mgr.delete_request(&input.request_id).await }
        })
}

fn add_env_cookie_and_workbench_handlers(handlers: Handlers, manager: &NetworkManager) -> Handlers {
    let m_save_env = manager.clone();
    let m_delete_env = manager.clone();
    let m_active_env = manager.clone();
    let m_global_vars = manager.clone();
    let m_save_cookie = manager.clone();
    let m_delete_cookie = manager.clone();
    let m_clear_cookies = manager.clone();
    let m_pick_spec = manager.clone();
    let m_preview_spec = manager.clone();
    let m_confirm_spec = manager.clone();
    let m_send = manager.clone();
    let m_cancel = manager.clone();
    let m_follow = manager.clone();

    handlers
        .command::<SaveEnvironmentCommand, _, _>(move |input: EnvironmentInput| {
            let mgr = m_save_env.clone();
            async move { mgr.save_environment(input).await }
        })
        .command::<DeleteEnvironmentCommand, _, _>(move |input: EnvironmentRef| {
            let mgr = m_delete_env.clone();
            async move { mgr.delete_environment(&input.environment_id).await }
        })
        .command::<SetActiveEnvironmentCommand, _, _>(move |input: SetActiveEnvironment| {
            let mgr = m_active_env.clone();
            async move {
                mgr.set_active_environment(&input.collection_id, input.environment_id.as_deref())
                    .await
            }
        })
        .command::<SaveGlobalVariablesCommand, _, _>(move |input: SaveGlobalVariables| {
            let mgr = m_global_vars.clone();
            async move { mgr.save_global_variables(input.variables).await }
        })
        .command::<SaveCookieCommand, _, _>(move |input: CookieInput| {
            let mgr = m_save_cookie.clone();
            async move { mgr.save_cookie(input).await }
        })
        .command::<DeleteCookieCommand, _, _>(move |input: CookieRef| {
            let mgr = m_delete_cookie.clone();
            async move { mgr.delete_cookie(&input.cookie_id).await }
        })
        .command::<ClearCookiesCommand, _, _>(move |()| {
            let mgr = m_clear_cookies.clone();
            async move { mgr.clear_cookies().await }
        })
        .command::<PickOpenApiFileCommand, _, _>(move |input: PickOpenApiFile| {
            let mgr = m_pick_spec.clone();
            async move { mgr.pick_openapi_file(&input.filter_name).await }
        })
        .command::<PreviewOpenApiTextCommand, _, _>(move |input: OpenApiTextInput| {
            std::future::ready(m_preview_spec.preview_openapi_text(&input.text))
        })
        .command::<ConfirmOpenApiImportCommand, _, _>(move |input: OpenApiConfirmInput| {
            let mgr = m_confirm_spec.clone();
            async move { mgr.confirm_openapi_import(input).await }
        })
        .command::<SendOwnerRequestCommand, _, _>(move |input: ManualSendInput| {
            let mgr = m_send.clone();
            async move { mgr.send_owner_request(input).await }
        })
        .command::<CancelOwnerRequestCommand, _, _>(move |input: CancelRunInput| {
            m_cancel.cancel_owner_request(&input.run_id);
            std::future::ready(Ok::<_, NetworkError>(()))
        })
        .channel::<FollowStateChannel, _, _>(move |()| {
            let stream = state_stream(m_follow.clone(), m_follow.subscribe());
            std::future::ready(Ok::<_, NetworkError>(stream))
        })
}

impl NetworkPlugin {
    async fn start_now(context: StartContext) -> Result<Started, StartFailure> {
        let store = context.service::<Store>()?;
        let secrets = context.service::<Secrets>()?;
        let file_dialog = context.service::<FileDialog>()?;
        let client = build_http_client().map_err(|_err| StartFailure::new(HTTP_FAILED))?;
        let manager = NetworkManager::load(store, secrets, file_dialog, context.clock(), client)
            .await
            .map_err(|error| match error {
                NetworkError::CredentialStoreUnavailable => {
                    StartFailure::new(CREDENTIAL_STORE_FAILED)
                }
                _other => StartFailure::new(STORAGE_FAILED),
            })?;

        let send_agents =
            AgentSelector::Agents(SEND_AGENTS.iter().map(|name| (*name).to_owned()).collect());
        let mention_provider: Arc<dyn MentionProvider> =
            Arc::new(NetworkMentions::new(manager.clone()));
        let contributions = Contributions::new()
            .add::<Tools>(PluginTool::new(
                Arc::new(ListCollectionsTool::new(manager.clone())),
                AgentSelector::Every,
            ))
            .add::<Tools>(PluginTool::new(
                Arc::new(ListRequestsTool::new(manager.clone())),
                AgentSelector::Every,
            ))
            .add::<Tools>(PluginTool::new(
                Arc::new(GetRequestTool::new(manager.clone())),
                AgentSelector::Every,
            ))
            .add::<Tools>(
                PluginTool::new(Arc::new(SendRequestTool::new(manager.clone())), send_agents)
                    .with_side_effects(),
            )
            .add::<Sections>(Section::new(
                SECTION_ID,
                SECTION_POSITION,
                SectionBody::Source(Arc::new(NetworkCollectionsSection::new(manager.clone()))),
            ))
            .add::<Mentions>(mention_provider)
            .add::<Management>(PluginManagement::new(
                NETWORK,
                Arc::new(NetworkManagement::new(manager.clone())),
            ));

        let ipc_handlers = handlers(&manager);
        let started = Started::new(NetworkInstance { manager })
            .with_contributions(contributions)
            .with_handlers(ipc_handlers);
        Ok(started)
    }
}

impl Plugin for NetworkPlugin {
    fn store_module(&self) -> Option<StoreModule> {
        Some(MODULE)
    }

    fn interface(&self) -> Interface {
        interface()
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        Box::pin(Self::start_now(context))
    }
}

#[derive(Debug)]
struct NetworkInstance {
    manager: NetworkManager,
}

impl Instance for NetworkInstance {
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        let count = self.manager.active_run_count();
        let items = if count == 0 {
            Vec::new()
        } else {
            vec![WorkItem::new(ACTIVE_REQUESTS, count)]
        };

        Box::pin(std::future::ready(items))
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        self.manager.cancel_all_runs();
        Box::pin(std::future::ready(()))
    }
}
