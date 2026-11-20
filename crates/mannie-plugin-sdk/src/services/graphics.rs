//! A native graphics viewport in the main window (ADR-0073 and ADR-0075). The desktop owns its
//! OpenGL context, checks the visible client area and releases the surface
//! when the channel closes, the plugin stops or the page reloads.

use std::fmt;
use std::io::{self, Write};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::service::Service;
use crate::{BoxFuture, BoxStream};

/// The desktop's native 2D drawing service.
#[derive(Debug)]
pub enum Graphics {}

impl Service for Graphics {
    const NAME: &'static str = "graphics";
    const MAJOR: u16 = 2;
    type Api = dyn GraphicsApi;
}

/// A native viewport belonging to one plugin start. All operations target
/// the main window. The plugin identifies its view and project.
pub trait GraphicsApi: Send + Sync + fmt::Debug {
    /// Opens a hidden surface. The returned stream owns it and reports its
    /// opaque lease once ready. Dropping the stream releases that lease.
    fn open(
        &self,
        scope: GraphicsView,
        scene: GraphicsScene,
    ) -> BoxFuture<'static, Result<BoxStream<'static, GraphicsEvent>, GraphicsError>>;

    /// Places the surface after clipping it to the trusted client area.
    fn place(
        &self,
        scope: GraphicsView,
        lease: String,
        placement: GraphicsPlacement,
    ) -> BoxFuture<'static, Result<(), GraphicsError>>;

    /// Replaces the scene, drawing only while the surface is visible. Callers keep
    /// one draw in flight per lease and replace pending previews with the latest snapshot.
    fn draw(
        &self,
        scope: GraphicsView,
        lease: String,
        scene: GraphicsScene,
    ) -> BoxFuture<'static, Result<(), GraphicsError>>;
}

/// The plugin's view and its currently displayed project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsView {
    /// The ID registered for the plugin's main view.
    pub view: String,
    /// An opaque project ID belonging to the plugin.
    pub project: String,
}

impl GraphicsView {
    /// Addresses a project in the named view.
    pub fn new(view: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            view: view.into(),
            project: project.into(),
        }
    }
}

/// A solid, opaque sRGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsColor {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
}

impl GraphicsColor {
    /// An opaque sRGB color given its three channel values.
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

/// Maximum absolute coordinate in document units for rectangle edges and view origins.
pub const DOCUMENT_LIMIT: f64 = 32_768.0;
/// Smallest accepted view magnification.
pub const MIN_ZOOM: f64 = 0.05;
/// Largest accepted view magnification.
pub const MAX_ZOOM: f64 = 64.0;
/// Maximum number of filled primitives in one scene.
pub const MAX_SHAPES: usize = 1024;
/// Maximum number of overlay primitives in one scene.
pub const MAX_OVERLAYS: usize = 1024;
/// Maximum length of the compact JSON representation of a scene.
pub const MAX_SCENE_BYTES: usize = 256 * 1024;
/// Width of a rectangle outline after projection to the device framebuffer.
pub const OUTLINE_DEVICE_PIXELS: f32 = 2.0;

/// The document point at the full viewport's top left and its magnification.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsTransform {
    /// Document coordinate on the horizontal axis.
    pub origin_x: f64,
    /// Document coordinate on the vertical axis.
    pub origin_y: f64,
    /// CSS pixels per document unit.
    pub zoom: f64,
}

impl GraphicsTransform {
    /// Places document point (0, 0) at the viewport origin with one CSS pixel per unit.
    pub const IDENTITY: Self = Self::new(0.0, 0.0, 1.0);

    /// A document origin and magnification.
    pub const fn new(origin_x: f64, origin_y: f64, zoom: f64) -> Self {
        Self {
            origin_x,
            origin_y,
            zoom,
        }
    }

    fn valid(self) -> bool {
        let bounds = -DOCUMENT_LIMIT..=DOCUMENT_LIMIT;
        bounds.contains(&self.origin_x)
            && bounds.contains(&self.origin_y)
            && (MIN_ZOOM..=MAX_ZOOM).contains(&self.zoom)
    }
}

/// An axis-aligned rectangle in document coordinates with an opaque color.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsRectangle {
    /// Left edge in document units.
    pub x: f64,
    /// Top edge in document units.
    pub y: f64,
    /// Positive width in document units.
    pub width: f64,
    /// Positive height in document units.
    pub height: f64,
    /// Fill or outline color.
    pub color: GraphicsColor,
}

impl GraphicsRectangle {
    /// A rectangle with the supplied document bounds and color.
    pub const fn new(x: f64, y: f64, width: f64, height: f64, color: GraphicsColor) -> Self {
        Self {
            x,
            y,
            width,
            height,
            color,
        }
    }

    fn valid(self) -> bool {
        let bounds = -DOCUMENT_LIMIT..=DOCUMENT_LIMIT;
        self.width > 0.0
            && self.height > 0.0
            && [self.x, self.y, self.x + self.width, self.y + self.height]
                .into_iter()
                .all(|edge| bounds.contains(&edge))
    }
}

/// A filled primitive, painted in list order over the background.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphicsShape {
    /// A solid rectangle.
    Rectangle(GraphicsRectangle),
}

/// A decoration, painted in list order over the complete shape layer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphicsOverlay {
    /// Strokes the rectangle's transformed edges with a two-device-pixel pen.
    RectangleOutline(GraphicsRectangle),
}

/// The complete drawing state for one frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsScene {
    /// The color behind all primitives.
    pub background: GraphicsColor,
    /// Maps document points to the full logical viewport.
    pub view: GraphicsTransform,
    /// Filled primitives in paint order.
    pub shapes: Vec<GraphicsShape>,
    /// Decorations in paint order.
    pub overlays: Vec<GraphicsOverlay>,
}

impl GraphicsScene {
    /// Replaces all layers and the view together.
    pub const fn new(
        background: GraphicsColor,
        view: GraphicsTransform,
        shapes: Vec<GraphicsShape>,
        overlays: Vec<GraphicsOverlay>,
    ) -> Self {
        Self {
            background,
            view,
            shapes,
            overlays,
        }
    }

    /// Checks geometry, list lengths and the compact JSON byte limit.
    pub fn valid(&self) -> bool {
        let shapes_fit = self.shapes.len() <= MAX_SHAPES;
        let overlays_fit = self.overlays.len() <= MAX_OVERLAYS;
        let valid_view = self.view.valid();
        if !(shapes_fit && overlays_fit && valid_view) {
            return false;
        }

        let shapes_valid = self
            .shapes
            .iter()
            .all(|GraphicsShape::Rectangle(rect)| rect.valid());
        let overlays_valid = self
            .overlays
            .iter()
            .all(|GraphicsOverlay::RectangleOutline(rect)| rect.valid());
        if !(shapes_valid && overlays_valid) {
            return false;
        }

        serde_json::to_writer(SceneBudget(MAX_SCENE_BYTES), self).is_ok()
    }
}

// Count exactly what serde writes without allocating a second copy of the scene.
struct SceneBudget(usize);

impl Write for SceneBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or(io::ErrorKind::FileTooLarge)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A rectangle measured from the webview's top left, in logical CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsRect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl GraphicsRect {
    /// A rectangle in the webview's logical coordinates.
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Bounds measurements before conversion to native coordinates. Partly
    /// clipped viewports can have negative origins.
    fn valid(self) -> bool {
        const LIMIT: f64 = 32_768.0;
        (-LIMIT..=LIMIT).contains(&self.x)
            && (-LIMIT..=LIMIT).contains(&self.y)
            && (0.0..=LIMIT).contains(&self.width)
            && (0.0..=LIMIT).contains(&self.height)
    }
}

/// A requested placement. The host intersects bounds, clip and client area.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsPlacement {
    /// The full viewport rectangle, before clipping.
    pub bounds: GraphicsRect,
    /// The visible crop, in the same coordinate system.
    pub clip: GraphicsRect,
    /// The webview's device pixel ratio.
    pub scale_factor: f64,
    /// False when an overlay or a hidden view owns this area.
    pub visible: bool,
}

impl GraphicsPlacement {
    /// Places the full viewport inside its visible crop at the supplied scale.
    pub const fn new(
        bounds: GraphicsRect,
        clip: GraphicsRect,
        scale_factor: f64,
        visible: bool,
    ) -> Self {
        Self {
            bounds,
            clip,
            scale_factor,
            visible,
        }
    }

    /// Whether the measurements and scale can be converted to native bounds.
    pub fn valid(&self) -> bool {
        self.bounds.valid() && self.clip.valid() && (0.25..=8.0).contains(&self.scale_factor)
    }
}

/// Status of the current lease. A failed stream ends after reporting the error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphicsEvent {
    /// The surface is ready to receive placement and scene updates.
    Ready {
        /// An opaque token valid only in this view, project and plugin start.
        lease: String,
    },
    /// The active surface can no longer present its scene.
    Failed {
        /// Reason for the failure.
        error: GraphicsError,
    },
}

/// Stable reasons for failures to open or update a viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum GraphicsError {
    /// This desktop has no graphics host for the platform.
    #[error("native graphics are unavailable on this platform")]
    UnsupportedPlatform,
    /// The graphics context could not be opened.
    #[error("the graphics context could not be opened")]
    GraphicsUnavailable,
    /// The host could not present the scene.
    #[error("the graphics frame could not be presented")]
    RenderFailed,
    /// The requested bounds or scale are invalid.
    #[error("the graphics placement is invalid")]
    InvalidPlacement,
    /// The lease expired or it belongs to a different scope.
    #[error("the graphics lease is no longer valid")]
    StaleLease,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn placement_allows_clipped_origins_and_empty_hidden_bounds() {
        let area = GraphicsRect {
            x: -20.0,
            y: 5.0,
            width: 80.0,
            height: 40.0,
        };
        let placement = GraphicsPlacement {
            bounds: area,
            clip: area,
            scale_factor: 1.25,
            visible: true,
        };

        assert!(placement.valid());
        assert!(
            GraphicsPlacement {
                clip: GraphicsRect { width: 0.0, ..area },
                visible: false,
                ..placement
            }
            .valid()
        );

        for invalid in [
            GraphicsPlacement {
                scale_factor: f64::NAN,
                ..placement
            },
            GraphicsPlacement {
                scale_factor: 0.0,
                ..placement
            },
            GraphicsPlacement {
                bounds: GraphicsRect {
                    width: -1.0,
                    ..area
                },
                ..placement
            },
            GraphicsPlacement {
                clip: GraphicsRect {
                    x: f64::INFINITY,
                    ..area
                },
                ..placement
            },
        ] {
            assert!(!invalid.valid());
        }
    }

    #[test]
    fn scenes_and_graphics_status_round_trip_through_the_protocol() {
        let scene = json!({
            "background": {"red": 33, "green": 33, "blue": 33},
            "view": {"originX": 0.0, "originY": -2.0, "zoom": 4.0},
            "shapes": [{
                "kind": "rectangle", "x": -2.0, "y": 1.0, "width": 60.0, "height": 80.0,
                "color": {"red": 0, "green": 255, "blue": 0}
            }],
            "overlays": [{
                "kind": "rectangle_outline", "x": -2.0, "y": 1.0, "width": 60.0, "height": 80.0,
                "color": {"red": 255, "green": 255, "blue": 255}
            }]
        });
        let drawing: GraphicsScene = serde_json::from_value(scene.clone()).unwrap();
        assert!(drawing.valid());
        assert_eq!(serde_json::to_value(drawing).unwrap(), scene);

        for event in [
            json!({"kind": "ready", "lease": "surface-1"}),
            json!({"kind": "failed", "error": {"code": "render_failed"}}),
        ] {
            let decoded: GraphicsEvent = serde_json::from_value(event.clone()).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), event);
        }
    }

    #[test]
    fn scene_rectangles_and_views_stay_within_the_document_bounds() {
        let white = GraphicsColor::new(255, 255, 255);
        let rectangle =
            GraphicsRectangle::new(-DOCUMENT_LIMIT, -DOCUMENT_LIMIT, 65_536.0, 65_536.0, white);
        assert!(rectangle.valid());

        for invalid in [
            GraphicsRectangle {
                x: -DOCUMENT_LIMIT - 1.0,
                ..rectangle
            },
            GraphicsRectangle {
                y: -DOCUMENT_LIMIT - 1.0,
                ..rectangle
            },
            GraphicsRectangle {
                width: rectangle.width + 1.0,
                ..rectangle
            },
            GraphicsRectangle {
                height: rectangle.height + 1.0,
                ..rectangle
            },
            GraphicsRectangle {
                width: 0.0,
                ..rectangle
            },
            GraphicsRectangle {
                height: -1.0,
                ..rectangle
            },
            GraphicsRectangle {
                x: f64::NAN,
                ..rectangle
            },
            GraphicsRectangle {
                width: f64::INFINITY,
                ..rectangle
            },
        ] {
            let shape = GraphicsScene::new(
                white,
                GraphicsTransform::IDENTITY,
                vec![GraphicsShape::Rectangle(invalid)],
                vec![],
            );
            let overlay = GraphicsScene::new(
                white,
                GraphicsTransform::IDENTITY,
                vec![],
                vec![GraphicsOverlay::RectangleOutline(invalid)],
            );
            assert!(!shape.valid());
            assert!(!overlay.valid());
        }

        for zoom in [MIN_ZOOM, MAX_ZOOM] {
            assert!(GraphicsTransform::new(-DOCUMENT_LIMIT, DOCUMENT_LIMIT, zoom).valid());
        }

        for invalid in [
            GraphicsTransform::new(-DOCUMENT_LIMIT - 1.0, 0.0, 1.0),
            GraphicsTransform::new(0.0, DOCUMENT_LIMIT + 1.0, 1.0),
            GraphicsTransform::new(0.0, 0.0, 0.049),
            GraphicsTransform::new(0.0, 0.0, 64.001),
            GraphicsTransform::new(f64::INFINITY, 0.0, 1.0),
            GraphicsTransform::new(0.0, 0.0, f64::NAN),
        ] {
            assert!(!GraphicsScene::new(white, invalid, vec![], vec![]).valid());
        }
    }

    #[test]
    fn list_limits_and_the_compact_json_budget_apply_independently() {
        let white = GraphicsColor::new(255, 255, 255);
        let rect = GraphicsRectangle::new(
            1.234_567_890_123_456,
            1.234_567_890_123_456,
            2.345_678_901_234_567,
            2.345_678_901_234_567,
            white,
        );
        let shape = GraphicsShape::Rectangle(rect);
        let outline = GraphicsOverlay::RectangleOutline(rect);
        let mut scene = GraphicsScene::new(white, GraphicsTransform::IDENTITY, vec![], vec![]);
        assert!(scene.valid());

        scene.shapes = vec![shape; MAX_SHAPES];
        assert!(scene.valid());
        scene.shapes.push(shape);
        assert!(!scene.valid());
        scene.shapes.clear();

        scene.overlays = vec![outline; MAX_OVERLAYS];
        assert!(scene.valid());
        scene.overlays.push(outline);
        assert!(!scene.valid());
        scene.overlays.pop();

        // Fill the remaining byte budget, then cross it with one more shape.
        let empty_shapes_len = serde_json::to_vec(&scene).unwrap().len();
        let per_shape = serde_json::to_vec(&shape).unwrap().len() + 1;
        let fits = (MAX_SCENE_BYTES - empty_shapes_len + 1) / per_shape;
        assert!((1..MAX_SHAPES).contains(&fits));
        scene.shapes = vec![shape; fits];
        assert!(serde_json::to_vec(&scene).unwrap().len() <= MAX_SCENE_BYTES);
        assert!(scene.valid());
        scene.shapes.push(shape);
        assert!(serde_json::to_vec(&scene).unwrap().len() > MAX_SCENE_BYTES);
        assert!(!scene.valid());
    }
}
