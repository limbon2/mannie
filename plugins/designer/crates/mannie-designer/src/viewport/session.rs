//! The status subscription owns the live edit target and its selection.
//! The host clips document geometry after transforming it (ADR-0075).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::ipc::PluginStream;
use mannie_plugin_sdk::services::graphics::{
    GraphicsApi, GraphicsColor, GraphicsEvent, GraphicsOverlay, GraphicsPlacement,
    GraphicsRectangle, GraphicsScene, GraphicsShape, GraphicsTransform, GraphicsView,
};
use tokio::sync::Mutex as AsyncMutex;

use super::{
    PlaceViewport, SetViewportColor, SetViewportSelection, ViewportError, ViewportGeometry,
    ViewportTarget,
};
use crate::document::{DocumentEdit, DocumentFailure, DocumentSnapshot};
use crate::plugin::{DESIGNER, Projects};

const BACKGROUND: GraphicsColor = GraphicsColor::new(32, 34, 38);
const OUTLINE: GraphicsColor = GraphicsColor::new(255, 255, 255);

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
    GraphicsScene::new(BACKGROUND, GraphicsTransform::IDENTITY, vec![], vec![])
}

fn validated_scene(scene: GraphicsScene) -> Result<GraphicsScene, ViewportError> {
    if !scene.valid() {
        return Err(ViewportError::RenderFailed);
    }

    Ok(scene)
}

#[derive(Default)]
struct Canvas {
    document: Option<DocumentSnapshot>,
    writable: bool,
    preview: Option<DocumentEdit>,
    selected: Option<String>,
    placement: Option<GraphicsPlacement>,
}

impl Canvas {
    fn scene(&self, edit: Option<DocumentEdit>) -> GraphicsScene {
        self.scene_with_selection(edit, self.selected.as_deref())
    }

    fn scene_with_selection(
        &self,
        edit: Option<DocumentEdit>,
        selected: Option<&str>,
    ) -> GraphicsScene {
        let mut scene = empty_scene();

        if let Some(document) = &self.document {
            let element = &document.element;
            let (geometry, hue) = match edit {
                Some(DocumentEdit::Geometry(geometry)) => (geometry, element.hue),
                Some(DocumentEdit::Hue(hue)) => (element.geometry(), hue),
                None => (element.geometry(), element.hue),
            };
            let rectangle = |color| {
                GraphicsRectangle::new(
                    geometry.x,
                    geometry.y,
                    geometry.width,
                    geometry.height,
                    color,
                )
            };
            let fill = color(hue);
            scene.shapes.push(GraphicsShape::Rectangle(rectangle(fill)));
            if selected == Some(element.id.as_str()) {
                scene
                    .overlays
                    .push(GraphicsOverlay::RectangleOutline(rectangle(OUTLINE)));
            }
        }

        scene
    }
}

struct Surface {
    scope: GraphicsView,
    lease: String,
    canvas: AsyncMutex<Canvas>,
}

impl Surface {
    async fn confirmed_edit<'a>(
        &self,
        graphics: &dyn GraphicsApi,
        canvas: &'a mut Canvas,
        element_id: &str,
        revision: u32,
    ) -> Result<&'a DocumentSnapshot, ViewportError> {
        let document = canvas
            .document
            .as_ref()
            .ok_or(ViewportError::StaleRevision)?;

        if canvas.writable && document.revision == revision && document.element.id == element_id {
            return Ok(document);
        }

        canvas.preview = None;
        canvas.writable = false;
        self.draw(graphics, canvas.scene(None)).await?;
        Err(ViewportError::StaleRevision)
    }

    async fn preview(
        &self,
        graphics: &dyn GraphicsApi,
        canvas: &mut Canvas,
        edit: DocumentEdit,
    ) -> Result<(), ViewportError> {
        self.draw(graphics, canvas.scene(Some(edit))).await?;
        canvas.preview = Some(edit);
        Ok(())
    }

    async fn draw(
        &self,
        graphics: &dyn GraphicsApi,
        scene: GraphicsScene,
    ) -> Result<(), ViewportError> {
        Ok(graphics
            .draw(
                self.scope.clone(),
                self.lease.clone(),
                validated_scene(scene)?,
            )
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
        let events = self
            .graphics
            .open(scope.clone(), validated_scene(empty_scene())?)
            .await?;

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

        if canvas.selected.as_ref() != canvas.document.as_ref().map(|doc| &doc.element.id) {
            canvas.selected = None;
        }

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
        canvas.placement = Some(placement);
        if canvas.document.is_none() {
            placement.visible = false;
        }

        surface.place(self.graphics.as_ref(), placement).await?;

        if !placement.visible && canvas.preview.take().is_some() {
            // Replace the hidden surface's draft before its next presentation.
            surface
                .draw(self.graphics.as_ref(), canvas.scene(None))
                .await?;
        }

        Ok(())
    }

    pub(super) async fn preview(&self, input: SetViewportColor) -> Result<(), ViewportError> {
        let surface = self.target(&input.project_id, &input.lease)?;
        let mut canvas = surface.canvas.lock().await;
        self.check(&surface)?;
        if !canvas.writable {
            return Err(ViewportError::StaleRevision);
        }

        surface
            .preview(
                self.graphics.as_ref(),
                &mut canvas,
                DocumentEdit::Hue(input.hue),
            )
            .await
    }

    pub(super) async fn preview_geometry(
        &self,
        input: ViewportGeometry,
    ) -> Result<(), ViewportError> {
        let surface = self.target(&input.project_id, &input.lease)?;
        let mut canvas = surface.canvas.lock().await;
        self.check(&surface)?;

        let graphics = self.graphics.as_ref();

        surface
            .confirmed_edit(graphics, &mut canvas, &input.element_id, input.revision)
            .await?;
        surface
            .preview(
                graphics,
                &mut canvas,
                DocumentEdit::Geometry(input.geometry()),
            )
            .await
    }

    pub(super) async fn select(&self, input: SetViewportSelection) -> Result<(), ViewportError> {
        let surface = self.target(&input.project_id, &input.lease)?;
        let mut canvas = surface.canvas.lock().await;
        self.check(&surface)?;

        let current = canvas
            .document
            .as_ref()
            .map(|document| &document.element.id);

        if !canvas.writable
            || input
                .element_id
                .as_ref()
                .is_some_and(|id| Some(id) != current)
        {
            return Err(ViewportError::StaleRevision);
        }
        if input.element_id == canvas.selected {
            return Ok(());
        }

        let preview = match (canvas.preview, &input.element_id) {
            (Some(DocumentEdit::Geometry(_)), None) => None,
            (preview, _) => preview,
        };
        let scene = canvas.scene_with_selection(preview, input.element_id.as_deref());

        surface.draw(self.graphics.as_ref(), scene).await?;
        self.check(&surface)?;
        canvas.preview = preview;
        canvas.selected = input.element_id;
        Ok(())
    }

    pub(super) async fn commit(
        &self,
        project_id: String,
        lease: &str,
        element_id: &str,
        revision: u32,
        edit: DocumentEdit,
    ) -> Result<DocumentSnapshot, ViewportError> {
        let surface = self.target(&project_id, lease)?;
        let mut canvas = surface.canvas.lock().await;
        let mut cache = self.confirmed.lock().await;
        self.check(&surface)?;

        let document = surface
            .confirmed_edit(self.graphics.as_ref(), &mut canvas, element_id, revision)
            .await?
            .clone();

        // The host checks its lease and that this scene can be presented
        // before the store admits the write.
        surface
            .draw(self.graphics.as_ref(), canvas.scene(Some(edit)))
            .await?;
        self.check(&surface)?;

        canvas.preview = None;
        if let DocumentEdit::Geometry(geometry) = edit
            && document.element.geometry() == geometry
        {
            return Ok(document);
        }

        let result = self
            .projects
            .store()
            .edit_document(&project_id, &document, edit)
            .await;

        match result {
            Ok(document) => {
                cache.insert(project_id, document.clone());
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
    use mannie_plugin_sdk::services::graphics::{
        GraphicsOverlay, GraphicsShape, GraphicsTransform,
    };

    use super::{Canvas, OUTLINE, ViewportError, validated_scene};
    use crate::document::{DocumentElement, DocumentSnapshot};

    #[test]
    fn shape_and_outline_keep_the_same_document_edges_outside_the_viewport() {
        let element = DocumentElement {
            x: -20.0,
            y: 75.0,
            width: 70.0,
            height: 80.0,
            ..DocumentElement::seed()
        };
        let selected = element.id.clone();
        let canvas = Canvas {
            document: Some(DocumentSnapshot {
                revision: 1,
                element,
            }),
            ..Canvas::default()
        };

        let scene = validated_scene(canvas.scene_with_selection(None, Some(&selected))).unwrap();
        assert_eq!(scene.view, GraphicsTransform::IDENTITY);
        let [GraphicsShape::Rectangle(fill)] = scene.shapes.as_slice() else {
            panic!("expected one rectangle");
        };
        let [GraphicsOverlay::RectangleOutline(outline)] = scene.overlays.as_slice() else {
            panic!("expected one rectangle outline");
        };

        for rect in [fill, outline] {
            assert_eq!(
                [rect.x, rect.y, rect.width, rect.height],
                [-20.0, 75.0, 70.0, 80.0]
            );
        }
        assert_eq!(outline.color, OUTLINE);
        let mut invalid = scene;
        let GraphicsShape::Rectangle(rect) = &mut invalid.shapes[0];
        rect.width = f64::INFINITY;
        assert!(matches!(
            validated_scene(invalid),
            Err(ViewportError::RenderFailed)
        ));
    }
}
