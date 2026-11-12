//! `DevTools` commands over the session's host, with deadlines and wiped
//! parameters.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroize, Zeroizing};

use super::ChromeError;
use super::host::{ChromeHost, ChromeHostFailure, ChromeReply};

const TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct Wire {
    host: Arc<dyn ChromeHost>,
    pub(super) closed: CancellationToken,
}

impl Wire {
    pub(super) fn new(host: Arc<dyn ChromeHost>) -> Self {
        Self {
            host,
            closed: CancellationToken::new(),
        }
    }

    pub(super) fn host(&self) -> &dyn ChromeHost {
        self.host.as_ref()
    }

    pub(super) async fn command(
        &self,
        tab: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, ChromeError> {
        self.submit(tab, method, params)?.wait().await
    }

    pub(super) fn submit(
        &self,
        tab: &str,
        method: &str,
        mut params: Value,
    ) -> Result<Call<'_>, ChromeError> {
        if self.closed.is_cancelled() {
            return Err(ChromeError::Disconnected);
        }
        let text = Zeroizing::new(
            serde_json::to_string(&params).map_err(|_json_error| ChromeError::Protocol)?,
        );
        wipe(&mut params);

        let observation = matches!(
            method,
            "Page.captureScreenshot" | "Accessibility.getFullAXTree" | "Page.getNavigationHistory"
        );
        let deadline = if observation {
            Duration::from_secs(2)
        } else if method.starts_with("Input.")
            || matches!(
                method,
                "Page.navigate"
                    | "Page.navigateToHistoryEntry"
                    | "Page.reload"
                    | "Runtime.callFunctionOn"
                    | "Network.setBypassServiceWorker"
                    | "Page.enable"
                    | "Page.setInterceptFileChooserDialog"
            )
        {
            Duration::from_secs(185)
        } else {
            TIMEOUT
        };
        let reply = match self.host.send(tab, method, text) {
            Ok(reply) => reply,
            Err(ChromeError::NoTab) => return Err(ChromeError::NoTab),
            Err(_) => {
                self.closed.cancel();
                return Err(ChromeError::Disconnected);
            }
        };
        Ok(Call {
            reply,
            deadline,
            observation,
            closed: &self.closed,
        })
    }
}

pub(super) struct Call<'a> {
    reply: ChromeReply,
    deadline: Duration,
    observation: bool,
    closed: &'a CancellationToken,
}

impl Call<'_> {
    pub(super) async fn wait(self) -> Result<Value, ChromeError> {
        let answer = tokio::select! {
            () = self.closed.cancelled() => return Err(ChromeError::Disconnected),
            answer = tokio::time::timeout(self.deadline, self.reply) => answer,
        };
        match answer {
            Ok(Ok(Ok(result))) if result.is_object() => Ok(result),
            Ok(Ok(Ok(_))) => Err(ChromeError::Protocol),
            Ok(Ok(Err(ChromeHostFailure::Protocol))) => Err(if self.observation {
                ChromeError::Stale
            } else {
                ChromeError::Protocol
            }),
            // The tab ended while the command ran, so its page is gone.
            Ok(Ok(Err(ChromeHostFailure::Disconnected)) | Err(_)) => Err(ChromeError::Disconnected),
            Err(_timeout) if self.observation && !self.closed.is_cancelled() => {
                Err(ChromeError::Stale)
            }
            Err(_timeout) => {
                // A command that never answers leaves the page ungoverned.
                self.closed.cancel();
                Err(ChromeError::Disconnected)
            }
        }
    }
}

fn wipe(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(wipe),
        Value::Object(object) => object.values_mut().for_each(wipe),
        _ => {}
    }
}
