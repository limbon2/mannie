//! The browser plugin's registrations and generated IPC through the host.

use std::future::ready;
use std::sync::Arc;

use mannie_browser::{UNSUPPORTED_PLATFORM, mannie_plugin};
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_harness::runtime::HarnessOptions;
use mannie_plugin_harness::types::PluginState;
use mannie_plugin_sdk::contracts::ConversationId;
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::points::tools::{BrowserAccess, BrowserAuthorization, BrowserControlAccess};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::browser::{
    Browser, BrowserApi, BrowserControl, BrowserCredential, BrowserError, BrowserFrame,
    BrowserPlacement, BrowserRequest, BrowserResult, BrowserStatus,
};
use mannie_plugin_sdk::{BoxFuture, BoxStream};

/// A browser service without sessions, on a system that can host them or
/// not.
#[derive(Debug, Clone, Copy)]
struct EmptyBrowser {
    supported: bool,
}

impl ServiceProvider for EmptyBrowser {
    fn name(&self) -> &'static str {
        Browser::NAME
    }

    fn major(&self) -> u16 {
        Browser::MAJOR
    }

    fn bind(&self, _binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        Ok(BoundService::new::<Browser>(Arc::new(*self)))
    }
}

impl BrowserApi for EmptyBrowser {
    fn supported(&self) -> bool {
        self.supported
    }

    fn status(
        &self,
        _conversation: ConversationId,
    ) -> BoxFuture<'static, Result<BrowserStatus, BrowserError>> {
        Box::pin(ready(Ok(BrowserStatus::default())))
    }

    fn watch(&self, _conversation: ConversationId) -> BoxStream<'static, BrowserStatus> {
        Box::pin(futures_util::stream::once(ready(BrowserStatus::default())))
    }

    fn place(
        &self,
        _conversation: ConversationId,
        _placement: BrowserPlacement,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        Box::pin(ready(Err(BrowserError::SessionMissing)))
    }

    fn focus(
        &self,
        _conversation: ConversationId,
        _tab: String,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        Box::pin(ready(Err(BrowserError::SessionMissing)))
    }

    fn still(
        &self,
        _conversation: ConversationId,
        _tab: String,
    ) -> BoxFuture<'static, Result<Option<BrowserFrame>, BrowserError>> {
        Box::pin(ready(Ok(None)))
    }

    fn control(
        &self,
        _conversation: ConversationId,
        _tab: Option<String>,
        _action: BrowserControl,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        Box::pin(ready(Err(BrowserError::SessionMissing)))
    }

    fn credential(
        &self,
        _conversation: ConversationId,
        _action: BrowserCredential,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        Box::pin(ready(Err(BrowserError::SessionMissing)))
    }

    fn trust_certificate(
        &self,
        _conversation: ConversationId,
        _session_id: String,
        _hostname: String,
        _fingerprint: String,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        Box::pin(ready(Err(BrowserError::SessionMissing)))
    }

    fn prepare(
        &self,
        _conversation: ConversationId,
        _request: BrowserRequest,
    ) -> BoxFuture<'static, Result<Vec<BrowserAccess>, BrowserError>> {
        Box::pin(ready(Err(BrowserError::SessionMissing)))
    }

    fn prepare_control(
        &self,
        _conversation: ConversationId,
    ) -> BoxFuture<'static, Result<Option<BrowserControlAccess>, BrowserError>> {
        Box::pin(ready(Err(BrowserError::SessionMissing)))
    }

    fn request_control(
        &self,
        _conversation: ConversationId,
        _authorization: Option<BrowserAuthorization>,
    ) -> BoxFuture<'static, Result<(), BrowserError>> {
        Box::pin(ready(Err(BrowserError::SessionMissing)))
    }

    fn execute(
        &self,
        _conversation: ConversationId,
        _request: BrowserRequest,
        _authorization: Option<BrowserAuthorization>,
    ) -> BoxFuture<'static, Result<BrowserResult, BrowserError>> {
        Box::pin(ready(Err(BrowserError::SessionMissing)))
    }
}

#[tokio::test]
async fn the_browser_plugin_conforms() {
    let options = HostOptions::new()
        .with_core(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .with_provider(Arc::new(EmptyBrowser { supported: true }));
    assert_eq!(
        conformance(mannie_plugin(), options).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn the_plugin_does_not_start_without_a_browser_engine() {
    let options = HostOptions::new()
        .with_core(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .with_provider(Arc::new(EmptyBrowser { supported: false }));
    let plugin = HostedPlugin::new(mannie_plugin(), options).await;
    let view = plugin.turn_on().await;
    assert_eq!(view.state, PluginState::Failed);
    assert_eq!(view.failure.as_deref(), Some(UNSUPPORTED_PLATFORM));
}
