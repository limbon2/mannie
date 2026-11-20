//! Window-scoped graphics leases for desktop plugins (ADR-0073). The registry
//! contains drawing data. The native objects stay on the window thread.

#[cfg(windows)]
mod windows;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::ids::{Contributor, owned_by};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::graphics::{
    Graphics, GraphicsApi, GraphicsError, GraphicsEvent, GraphicsPlacement, GraphicsScene,
    GraphicsView,
};
use mannie_plugin_sdk::{BoxFuture, BoxStream};
use tauri::AppHandle;
use tokio::sync::{mpsc, oneshot};

#[cfg(feature = "validation")]
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphicsSurfaceReport {
    pub(crate) visible: bool,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) center: Option<[u8; 3]>,
    pub(crate) corner: Option<[u8; 3]>,
    pub(crate) outline_edge: Option<[[u8; 3]; 4]>,
}

#[cfg(not(windows))]
mod platform {
    use super::{AppHandle, Arc, GraphicsError, GraphicsPlacement, GraphicsScene};

    pub(super) fn open(
        _app: &AppHandle,
        _id: u64,
        _failed: Arc<dyn Fn(GraphicsError) + Send + Sync>,
    ) -> Result<(), GraphicsError> {
        Err(GraphicsError::UnsupportedPlatform)
    }

    pub(super) fn render(
        _id: u64,
        _placement: &GraphicsPlacement,
        _scene: Arc<GraphicsScene>,
    ) -> Result<(), GraphicsError> {
        Err(GraphicsError::UnsupportedPlatform)
    }

    pub(super) fn close(_id: u64) {}

    #[cfg(feature = "validation")]
    pub(super) fn reports() -> Vec<super::GraphicsSurfaceReport> {
        Vec::new()
    }
}

#[cfg(windows)]
use windows as platform;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Send the result back without blocking either the runtime or the UI thread.
async fn on_window<T: Send + 'static>(
    app: &AppHandle,
    action: impl FnOnce() -> Result<T, GraphicsError> + Send + 'static,
) -> Result<T, GraphicsError> {
    let (send, receive) = oneshot::channel();
    app.run_on_main_thread(move || {
        let _sent = send.send(action());
    })
    .map_err(|_closed| GraphicsError::GraphicsUnavailable)?;
    receive
        .await
        .map_err(|_cancelled| GraphicsError::GraphicsUnavailable)?
}

#[derive(Debug)]
pub(crate) struct GraphicsProvider {
    app: AppHandle,
    bindings: Mutex<HashMap<Contributor, BoundGraphics>>,
}

impl GraphicsProvider {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self {
            app,
            bindings: Mutex::default(),
        }
    }

    pub(crate) fn page_loaded(&self) {
        for binding in lock(&self.bindings).values() {
            binding.reset(false);
        }
    }

    #[cfg(feature = "validation")]
    pub(crate) async fn surfaces(&self) -> Result<Vec<GraphicsSurfaceReport>, GraphicsError> {
        on_window(&self.app, || Ok(platform::reports())).await
    }
}

impl ServiceProvider for GraphicsProvider {
    fn name(&self) -> &'static str {
        Graphics::NAME
    }

    fn major(&self) -> u16 {
        Graphics::MAJOR
    }

    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        let contributor = binding.contributor();
        let graphics = BoundGraphics {
            app: self.app.clone(),
            contributor: contributor.clone(),
            state: Arc::new(Mutex::new(BindingState::default())),
        };
        let previous = lock(&self.bindings).insert(contributor, graphics.clone());

        if let Some(previous) = previous {
            previous.reset(true);
        }

        Ok(BoundService::new::<Graphics>(Arc::new(graphics)))
    }

    fn release(&self, contributor: &Contributor) -> BoxFuture<'static, ()> {
        let binding = lock(&self.bindings).remove(contributor);

        if let Some(binding) = binding {
            binding.reset(true);
        }

        Box::pin(std::future::ready(()))
    }
}

#[derive(Debug, Clone)]
struct BoundGraphics {
    app: AppHandle,
    contributor: Contributor,
    state: Arc<Mutex<BindingState>>,
}

#[derive(Debug, Default)]
struct BindingState {
    revoked: bool,
    surface: Option<Surface>,
}

#[derive(Debug)]
struct Surface {
    id: u64,
    token: String,
    scope: GraphicsView,
    scene: Arc<GraphicsScene>,
    placement: Option<GraphicsPlacement>,
    failed: mpsc::Sender<GraphicsError>,
}

impl BindingState {
    fn current(
        &mut self,
        scope: &GraphicsView,
        token: &str,
    ) -> Result<&mut Surface, GraphicsError> {
        self.surface
            .as_mut()
            .filter(|surface| surface.scope == *scope && surface.token == token)
            .ok_or(GraphicsError::StaleLease)
    }

    fn take_if(&mut self, id: u64) -> Option<Surface> {
        self.surface.take_if(|surface| surface.id == id)
    }
}

/// Created before posting an open. Cancellation during initialization releases
/// the native child just as dropping the established status stream does.
struct Lease {
    binding: BoundGraphics,
    id: u64,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let surface = lock(&self.binding.state).take_if(self.id);
        self.binding.close(surface);
    }
}

impl BoundGraphics {
    fn close(&self, surface: Option<Surface>) {
        if let Some(surface) = surface {
            let _sent = surface.failed.try_send(GraphicsError::StaleLease);
            let id = surface.id;
            let _closed = self.app.run_on_main_thread(move || platform::close(id));
        }
    }

    fn reset(&self, revoke: bool) {
        let surface = {
            let mut state = lock(&self.state);
            state.revoked |= revoke;
            state.surface.take()
        };
        self.close(surface);
    }

    fn update(
        &self,
        scope: GraphicsView,
        token: String,
        change: impl FnOnce(&mut Surface) -> Result<(), GraphicsError> + Send + 'static,
    ) -> BoxFuture<'static, Result<(), GraphicsError>> {
        let binding = self.clone();
        Box::pin(async move {
            on_window(&binding.app, move || {
                let (id, placement, scene, failed) = {
                    let mut state = lock(&binding.state);
                    let surface = state.current(&scope, &token)?;
                    change(surface)?;
                    (
                        surface.id,
                        surface.placement,
                        Arc::clone(&surface.scene),
                        surface.failed.clone(),
                    )
                };

                if let Some(placement) = placement {
                    platform::render(id, &placement, scene).inspect_err(|error| {
                        let _reported = failed.try_send(*error);
                    })?;
                }

                Ok(())
            })
            .await
        })
    }

    /// Called on the first poll of an admitted page's status stream. A request
    /// from an old page can finish opening its IPC channel after reload. The
    /// subscription registry drops that stream without ever polling it.
    async fn acquire(
        self,
        scope: GraphicsView,
        scene: GraphicsScene,
    ) -> Result<BoxStream<'static, GraphicsEvent>, GraphicsError> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);

        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let token = uuid::Uuid::now_v7().to_string();
        let (failed, mut failures) = mpsc::channel(1);

        let old = {
            let mut state = lock(&self.state);

            if state.revoked {
                return Err(GraphicsError::StaleLease);
            }

            state.surface.replace(Surface {
                id,
                token: token.clone(),
                scope,
                scene: Arc::new(scene),
                placement: None,
                failed: failed.clone(),
            })
        };

        self.close(old);

        let lease = Lease {
            binding: self.clone(),
            id,
        };

        let app = self.app.clone();

        on_window(&self.app, move || {
            let stale = lock(&self.state)
                .surface
                .as_ref()
                .is_none_or(|surface| surface.id != id);

            if stale {
                return Err(GraphicsError::StaleLease);
            }

            platform::open(
                &app,
                id,
                Arc::new(move |error| {
                    let _reported = failed.try_send(error);
                }),
            )
        })
        .await?;

        let ready = stream::iter([GraphicsEvent::Ready { lease: token }]);
        let terminal = stream::once(async move {
            let error = failures.recv().await.unwrap_or(GraphicsError::StaleLease);
            drop(lease);
            GraphicsEvent::Failed { error }
        });

        Ok(ready.chain(terminal).boxed())
    }
}

impl GraphicsApi for BoundGraphics {
    fn open(
        &self,
        scope: GraphicsView,
        scene: GraphicsScene,
    ) -> BoxFuture<'static, Result<BoxStream<'static, GraphicsEvent>, GraphicsError>> {
        let valid_view =
            scope.view.len() <= 128 && owned_by(self.contributor.plugin_id().as_str(), &scope.view);
        let valid_project = (1..=256).contains(&scope.project.len());

        if !valid_view || !valid_project || !scene.valid() {
            return Box::pin(std::future::ready(Err(GraphicsError::InvalidPlacement)));
        }

        let binding = self.clone();
        let stream = stream::once(async move {
            binding
                .acquire(scope, scene)
                .await
                .unwrap_or_else(|error| stream::iter([GraphicsEvent::Failed { error }]).boxed())
        })
        .flatten()
        .boxed();

        Box::pin(std::future::ready(Ok(stream)))
    }

    fn place(
        &self,
        scope: GraphicsView,
        lease: String,
        placement: GraphicsPlacement,
    ) -> BoxFuture<'static, Result<(), GraphicsError>> {
        self.update(scope, lease, move |surface| {
            if !placement.valid() {
                return Err(GraphicsError::InvalidPlacement);
            }

            surface.placement = Some(placement);
            Ok(())
        })
    }

    fn draw(
        &self,
        scope: GraphicsView,
        lease: String,
        scene: GraphicsScene,
    ) -> BoxFuture<'static, Result<(), GraphicsError>> {
        if !scene.valid() {
            return Box::pin(std::future::ready(Err(GraphicsError::InvalidPlacement)));
        }

        self.update(scope, lease, move |surface| {
            surface.scene = Arc::new(scene);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mannie_plugin_sdk::services::graphics::{GraphicsColor, GraphicsTransform};

    #[test]
    fn replaced_project_cannot_draw_move_or_close_its_successor() {
        let red = GraphicsColor::new(255, 0, 0);
        let scope = GraphicsView::new("designer", "second");
        let scene = GraphicsScene::new(red, GraphicsTransform::IDENTITY, vec![], vec![]);
        let (failed, _errors) = mpsc::channel(1);
        let mut state = BindingState {
            revoked: false,
            surface: Some(Surface {
                id: 2,
                token: "new".into(),
                scope: scope.clone(),
                scene: Arc::new(scene),
                placement: None,
                failed,
            }),
        };
        let previous = GraphicsView::new(&scope.view, "first");
        assert!(matches!(
            state.current(&scope, "old"),
            Err(GraphicsError::StaleLease)
        ));
        assert!(matches!(
            state.current(&previous, "new"),
            Err(GraphicsError::StaleLease)
        ));
        assert!(state.take_if(1).is_none());
        assert_eq!(state.current(&scope, "new").unwrap().id, 2);
        assert!(state.take_if(2).is_some());
        assert!(matches!(
            state.current(&scope, "new"),
            Err(GraphicsError::StaleLease)
        ));
    }
}
