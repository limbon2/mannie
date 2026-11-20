//! Records what Designer sends to the desktop graphics service.

use std::future::ready;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use futures_util::{StreamExt as _, stream};
use mannie_plugin_harness::host::HostOptions;
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::graphics::{
    Graphics, GraphicsApi, GraphicsError, GraphicsEvent, GraphicsPlacement, GraphicsScene,
    GraphicsView,
};
use mannie_plugin_sdk::{BoxFuture, BoxStream};
use tokio::sync::oneshot;

#[derive(Debug, PartialEq)]
pub(super) enum Call {
    Open(GraphicsView, GraphicsScene),
    Draw(GraphicsView, String, GraphicsScene),
    Place(GraphicsView, String, GraphicsPlacement),
}

#[derive(Debug, Clone, Default)]
pub(super) struct RecordingGraphics {
    calls: Arc<Mutex<Vec<Call>>>,
    open_error: Option<GraphicsError>,
    leases: Arc<AtomicUsize>,
    opening: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
    fail_draw: Arc<AtomicBool>,
}

impl RecordingGraphics {
    pub(super) fn fail_next_draw(&self) {
        self.fail_draw.store(true, Ordering::Relaxed);
    }

    pub(super) fn delay_next_ready(&self) -> oneshot::Sender<()> {
        let (send, receive) = oneshot::channel();
        *self
            .opening
            .lock()
            .expect("the graphics recorder is available") = Some(receive);
        send
    }

    pub(super) fn unavailable(error: GraphicsError) -> Self {
        Self {
            open_error: Some(error),
            ..Self::default()
        }
    }

    pub(super) fn options(&self) -> HostOptions {
        HostOptions::new().with_provider(Arc::new(self.clone()))
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        std::mem::take(&mut *self.recording())
    }

    fn recording(&self) -> MutexGuard<'_, Vec<Call>> {
        self.calls
            .lock()
            .expect("the graphics recorder is available")
    }
}

impl ServiceProvider for RecordingGraphics {
    fn name(&self) -> &'static str {
        Graphics::NAME
    }

    fn major(&self) -> u16 {
        Graphics::MAJOR
    }

    fn bind(&self, _binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        Ok(BoundService::new::<Graphics>(Arc::new(self.clone())))
    }
}

impl GraphicsApi for RecordingGraphics {
    fn open(
        &self,
        scope: GraphicsView,
        scene: GraphicsScene,
    ) -> BoxFuture<'static, Result<BoxStream<'static, GraphicsEvent>, GraphicsError>> {
        self.recording().push(Call::Open(scope, scene));

        if let Some(error) = self.open_error {
            return Box::pin(ready(Err(error)));
        }

        let leases = Arc::clone(&self.leases);
        let opening = self
            .opening
            .lock()
            .expect("the graphics recorder is available")
            .take();
        let acquire = async move {
            let number = leases.fetch_add(1, Ordering::Relaxed) + 1;

            if let Some(opening) = opening {
                opening
                    .await
                    .expect("the test releases the waiting surface");
            }

            GraphicsEvent::Ready {
                lease: format!("lease-{number}"),
            }
        };
        let lease = stream::once(acquire).chain(stream::pending()).boxed();
        Box::pin(ready(Ok(lease)))
    }

    fn place(
        &self,
        scope: GraphicsView,
        lease: String,
        placement: GraphicsPlacement,
    ) -> BoxFuture<'static, Result<(), GraphicsError>> {
        self.recording().push(Call::Place(scope, lease, placement));
        Box::pin(ready(Ok(())))
    }

    fn draw(
        &self,
        scope: GraphicsView,
        lease: String,
        scene: GraphicsScene,
    ) -> BoxFuture<'static, Result<(), GraphicsError>> {
        if lease != format!("lease-{}", self.leases.load(Ordering::Relaxed)) {
            return Box::pin(ready(Err(GraphicsError::StaleLease)));
        }
        if self.fail_draw.swap(false, Ordering::Relaxed) {
            return Box::pin(ready(Err(GraphicsError::RenderFailed)));
        }

        self.recording().push(Call::Draw(scope, lease, scene));
        Box::pin(ready(Ok(())))
    }
}
