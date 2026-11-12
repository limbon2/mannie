//! Browser tools and the Browser pane (ADR-0064, ADR-0065).

mod commands;
mod tools;

use std::sync::Arc;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::StartContext;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::Interface;
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::tools::{AgentSelector, PluginTool, Tools};
use mannie_plugin_sdk::services::browser::Browser;

use crate::tools::{BrowserTool, BrowserToolKind};

/// The code of a start on a system without a usable browser engine, which
/// the plugin's UI words on the Plugins page.
pub const UNSUPPORTED_PLATFORM: &str = "unsupported_platform";

/// The Browser plugin definition.
#[derive(Debug, Default)]
pub struct BrowserPlugin;

impl BrowserPlugin {
    /// The Browser plugin, off until the owner enables it.
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for BrowserPlugin {
    fn interface(&self) -> Interface {
        commands::interface()
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        Box::pin(async move {
            let browser = context.service::<Browser>()?;
            // Browser needs a Chromium webview with a DevTools endpoint, which
            // only WebView2 on Windows offers (ADR-0064, decision 1).
            if !browser.supported() {
                return Err(StartFailure::new(UNSUPPORTED_PLATFORM));
            }
            let mut contributions = Contributions::new();
            for kind in [
                BrowserToolKind::Tabs,
                BrowserToolKind::Navigate,
                BrowserToolKind::Observe,
                BrowserToolKind::Interact,
                BrowserToolKind::RequestControl,
            ] {
                let tool = BrowserTool::new(Arc::clone(&browser), kind);
                let tool = PluginTool::new(Arc::new(tool), AgentSelector::Conversations);
                let tool = match kind {
                    BrowserToolKind::Observe => tool.with_browser_reads(),
                    BrowserToolKind::Tabs
                    | BrowserToolKind::Navigate
                    | BrowserToolKind::Interact
                    | BrowserToolKind::RequestControl => tool.with_browser_actions(),
                };
                contributions = contributions.add::<Tools>(tool);
            }
            Ok(Started::new(RunningBrowser)
                .with_contributions(contributions)
                .with_handlers(commands::handlers(&browser)))
        })
    }
}

#[derive(Debug)]
struct RunningBrowser;

impl Instance for RunningBrowser {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        // The host releases the browser service and closes its sessions.
        Box::pin(std::future::ready(()))
    }
}

mannie_plugin_sdk::export_plugin!(BrowserPlugin::new);
