//! The status subscription owns the live edit target. Graphics v1 receives
//! only the visible part of the drawing, normalized to the full viewport.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::ipc::PluginStream;
use mannie_plugin_sdk::services::graphics::{
    GraphicsApi, GraphicsColor, GraphicsEvent, GraphicsPlacement, GraphicsRectangle, GraphicsScene,
    GraphicsView,
};
use tokio::sync::Mutex as AsyncMutex;

use super::{CommitViewportColor, PlaceViewport, SetViewportColor, ViewportError, ViewportTarget};
use crate::document::{DocumentElement, DocumentFailure, DocumentSnapshot};
use crate::plugin::{DESIGNER, Projects};

const BACKGROUND: GraphicsColor = GraphicsColor::new(32, 34, 38);

/// Full saturation and value, including both red endpoints of the slider.
fn color(hue: u16) -> GraphicsColor {
    let hue = hue % 360;
    let rising = u8::try_from(((hue % 60) * 255 + 30) / 60)
        .expect("a fraction of a color channel fits in a byte");
    let falling = 255 - rising;
    let (red, green, blue) = match hue / 60 {
        0 => (255, rising, 0),
        1 => (falling, 255, 0),
        2 => (0, 255, rising),
        3 => (0, falling, 255),
        4 => (rising, 0, 255),
        _ => (255, 0, falling),
    };

    GraphicsColor::new(red, green, blue)
}

fn empty_scene() -> GraphicsScene {
    GraphicsScene::new(
        BACKGROUND,
        GraphicsRectangle::new(0.0, 0.0, 0.0, 0.0, BACKGROUND),
    )
}

fn project_axis(position: f64, length: f64, viewport: f64) -> (f64, f64) {
    if viewport <= 0.0 {
        return (0.0, 0.0);
    }

    let start = (position / viewport).clamp(0.0, 1.0);
    let end = ((position + length) / viewport).clamp(0.0, 1.0);
    (start, end - start)
}

fn scene(element: &DocumentElement, hue: u16, placement: GraphicsPlacement) -> GraphicsScene {
    let (x, width) = project_axis(element.x, element.width, placement.bounds.width);
    let (y, height) = project_axis(element.y, element.height, placement.bounds.height);
    GraphicsScene::new(
        BACKGROUND,
        GraphicsRectangle::new(x, y, width, height, color(hue)),
    )
}

#[derive(Default)]
struct Canvas {
    document: Option<DocumentSnapshot>,
    writable: bool,
    preview: Option<u16>,
    placement: Option<GraphicsPlacement>,
}

impl Canvas {
    fn scene(&self, hue: Option<u16>) -> GraphicsScene {
        match (&self.document, self.placement) {
            (Some(document), Some(placement)) => {
                let hue = hue.unwrap_or(document.element.hue);
                scene(&document.element, hue, placement)
            }
            _ => empty_scene(),
        }
    }
}

struct Surface {
    scope: GraphicsView,
    lease: String,
    canvas: AsyncMutex<Canvas>,
}

impl Surface {
    async fn draw(
        &self,
        graphics: &dyn GraphicsApi,
        scene: GraphicsScene,
    ) -> Result<(), ViewportError> {
        Ok(graphics
            .draw(self.scope.clone(), self.lease.clone(), scene)
            .await?)
    }

    async fn place(
        &self,
        graphics: &dyn GraphicsApi,
        placement: GraphicsPlacement,
    ) -> Result<(), ViewportError> {
        Ok(graphics
            .place(self.scope.clone(), self.lease.clone(), placement)
            .await?)
    }
}

type ActiveSurface = Arc<Mutex<Option<Arc<Surface>>>>;

fn active(surface: &ActiveSurface) -> MutexGuard<'_, Option<Arc<Surface>>> {
    surface.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Dropping a subscription revokes edit authority even if a command is
/// waiting for the previous operation or the native service to finish.
struct Subscription {
    active: ActiveSurface,
    scope: GraphicsView,
    lease: Option<String>,
}

impl Subscription {
    fn receive(&mut self, event: &GraphicsEvent) {
        match event {
            GraphicsEvent::Ready { lease } => {
                self.lease = Some(lease.clone());
                *active(&self.active) = Some(Arc::new(Surface {
                    scope: self.scope.clone(),
                    lease: lease.clone(),
                    canvas: AsyncMutex::default(),
                }));
            }
            GraphicsEvent::Failed { .. } => self.revoke(),
        }
    }

    fn revoke(&mut self) {
        let mut current = active(&self.active);

        if current
            .as_ref()
            .is_some_and(|surface| Some(&surface.lease) == self.lease.as_ref())
        {
            current.take();
        }

        self.lease = None;
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.revoke();
    }
}

pub(super) struct Viewports {
    projects: Arc<Projects>,
    graphics: Arc<dyn GraphicsApi>,
    active: ActiveSurface,
    // Serialize storage across a lease replacement. A newly opened lease
    // must observe any commit admitted before the old lease closed.
    confirmed: Arc<AsyncMutex<HashMap<String, DocumentSnapshot>>>,
}

impl Viewports {
    pub(super) fn new(projects: Arc<Projects>, graphics: Arc<dyn GraphicsApi>) -> Self {
        Self {
            projects,
            graphics,
            active: ActiveSurface::default(),
            confirmed: Arc::default(),
        }
    }

    pub(super) async fn open(
        &self,
        project: String,
    ) -> Result<PluginStream<GraphicsEvent>, ViewportError> {
        if !self.projects.contains(&project) {
            return Err(ViewportError::ProjectMissing);
        }

        let scope = GraphicsView::new(DESIGNER, project);
        let events = self.graphics.open(scope.clone(), empty_scene()).await?;
        let subscription = Subscription {
            active: Arc::clone(&self.active),
            scope,
            lease: None,
        };
        // The desktop replaces its native lease on the first poll. Serialize
        // that poll through Ready with other opens and document operations.
        // An abandoned, never-polled subscription owns no graphics resources.
        let opening = Some(Arc::clone(&self.confirmed));
        Ok(stream::unfold(
            (events, subscription, opening),
            |(mut events, mut subscription, opening)| async move {
                let _acquiring = match opening {
                    Some(documents) => {
                        let guard = documents.lock_owned().await;
                        active(&subscription.active).take();
                        Some(guard)
                    }
                    None => None,
                };
                let event = events.next().await?;
                subscription.receive(&event);
                Some((event, (events, subscription, None)))
            },
        )
        .boxed())
    }

    fn target(&self, project: &str, lease: &str) -> Result<Arc<Surface>, ViewportError> {
        active(&self.active)
            .as_ref()
            .filter(|surface| surface.scope.project == project && surface.lease == lease)
            .cloned()
            .ok_or(ViewportError::StaleLease)
    }

    fn check(&self, surface: &Surface) -> Result<(), ViewportError> {
        self.target(&surface.scope.project, &surface.lease)
            .map(drop)
    }

    pub(super) async fn read(
        &self,
        input: ViewportTarget,
    ) -> Result<DocumentSnapshot, ViewportError> {
        let surface = self.target(&input.project_id, &input.lease)?;
        let mut canvas = surface.canvas.lock().await;
        let mut cache = self.confirmed.lock().await;
        self.check(&surface)?;

        let result = self
            .projects
            .store()
            .document(&input.project_id, self.projects.now())
            .await;

        if let Ok(document) = &result {
            cache.insert(input.project_id.clone(), document.clone());
        }

        canvas.document = cache.get(&input.project_id).cloned();
        drop(cache);
        self.check(&surface)?;
        canvas.preview = None;
        canvas.writable = result.is_ok();

        if canvas.document.is_some() {
            surface
                .draw(self.graphics.as_ref(), canvas.scene(None))
                .await?;

            if let Some(placement) = canvas.placement {
                surface.place(self.graphics.as_ref(), placement).await?;
            }
        }

        result.map_err(|failure| document_error(failure, &canvas))
    }

    pub(super) async fn place(&self, input: PlaceViewport) -> Result<(), ViewportError> {
        let surface = self.target(&input.project_id, &input.lease)?;
        let mut canvas = surface.canvas.lock().await;
        self.check(&surface)?;

        let mut placement = input.placement;
        let refresh = canvas.placement.is_none_or(|old| {
            !old.visible
                || (old.bounds.width, old.bounds.height)
                    != (placement.bounds.width, placement.bounds.height)
        });
        canvas.placement = Some(placement);
        if canvas.document.is_none() {
            placement.visible = false;
        }
        if !placement.visible {
            canvas.preview = None;
        } else if refresh {
            surface
                .draw(self.graphics.as_ref(), canvas.scene(canvas.preview))
                .await?;
        }

        surface.place(self.graphics.as_ref(), placement).await
    }

    pub(super) async fn preview(&self, input: SetViewportColor) -> Result<(), ViewportError> {
        let surface = self.target(&input.project_id, &input.lease)?;
        let mut canvas = surface.canvas.lock().await;
        self.check(&surface)?;
        if !canvas.writable {
            return Err(ViewportError::StaleRevision);
        }

        surface
            .draw(self.graphics.as_ref(), canvas.scene(Some(input.hue)))
            .await?;
        canvas.preview = Some(input.hue);
        Ok(())
    }

    pub(super) async fn commit(
        &self,
        input: CommitViewportColor,
    ) -> Result<DocumentSnapshot, ViewportError> {
        let surface = self.target(&input.project_id, &input.lease)?;
        let mut canvas = surface.canvas.lock().await;
        let mut cache = self.confirmed.lock().await;
        self.check(&surface)?;

        let document = canvas
            .document
            .as_ref()
            .ok_or(ViewportError::StaleRevision)?;

        if !canvas.writable
            || document.revision != input.revision
            || document.element.id != input.element_id
        {
            canvas.preview = None;
            canvas.writable = false;
            surface
                .draw(self.graphics.as_ref(), canvas.scene(None))
                .await?;
            return Err(ViewportError::StaleRevision);
        }

        // The host checks its lease and that this scene can be presented
        // before the store admits the write.
        surface
            .draw(self.graphics.as_ref(), canvas.scene(Some(input.hue)))
            .await?;
        self.check(&surface)?;

        let result = self
            .projects
            .store()
            .set_document_hue(&input.project_id, document, input.hue)
            .await;
        canvas.preview = None;
        match result {
            Ok(document) => {
                cache.insert(input.project_id, document.clone());
                canvas.document = Some(document.clone());
                Ok(document)
            }
            Err(failure) => {
                drop(cache);
                canvas.writable = false;
                surface
                    .draw(self.graphics.as_ref(), canvas.scene(None))
                    .await?;
                Err(document_error(failure, &canvas))
            }
        }
    }
}

fn document_error(failure: DocumentFailure, canvas: &Canvas) -> ViewportError {
    match failure {
        DocumentFailure::ProjectMissing => ViewportError::ProjectMissing,
        DocumentFailure::StaleRevision => ViewportError::StaleRevision,
        DocumentFailure::StorageFailed => ViewportError::StorageFailed {
            confirmed: canvas.document.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::services::graphics::{GraphicsPlacement, GraphicsRect};

    use super::{project_axis, scene};
    use crate::document::DocumentElement;

    #[test]
    fn projection_clips_to_full_logical_bounds_even_with_a_scaled_and_cropped_surface() {
        let position = DocumentElement {
            x: -20.0,
            y: 75.0,
            width: 70.0,
            height: 80.0,
            ..DocumentElement::seed()
        };
        let bounds = GraphicsRect::new(-10.0, 2.0, 200.0, 100.0);
        let crop = GraphicsRect::new(90.0, 30.0, 30.0, 45.0);
        let placement = GraphicsPlacement::new(bounds, crop, 2.5, true);
        let drawing = scene(&position, 360, placement);
        assert!(drawing.valid());
        let rect = drawing.rectangle;
        assert_eq!(
            [rect.x, rect.y, rect.width, rect.height],
            [0.0, 0.75, 0.25, 0.25]
        );
        assert_eq!(project_axis(-40.0, 10.0, 100.0), (0.0, 0.0));
        assert_eq!(project_axis(140.0, 10.0, 100.0), (1.0, 0.0));
        assert_eq!(project_axis(40.0, 10.0, 0.0), (0.0, 0.0));
    }
}
