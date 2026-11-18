//! A native graphics viewport in the main window (ADR-0073). The desktop owns its
//! OpenGL context, checks the visible client area and releases the surface
//! when the channel closes, the plugin stops or the page reloads.

use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::service::Service;
use crate::{BoxFuture, BoxStream};

/// The desktop's native 2D drawing service.
#[derive(Debug)]
pub enum Graphics {}

impl Service for Graphics {
    const NAME: &'static str = "graphics";
    const MAJOR: u16 = 1;
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

    /// Replaces the scene, drawing only while the surface is visible.
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

/// A filled rectangle in normalized viewport coordinates, measured from
/// the top left of the full viewport before clipping.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsRectangle {
    /// Left edge as a fraction of the viewport width.
    pub x: f64,
    /// Top edge as a fraction of the viewport height.
    pub y: f64,
    /// Fraction of the viewport width.
    pub width: f64,
    /// Fraction of the viewport height.
    pub height: f64,
    /// Fill color.
    pub color: GraphicsColor,
}

impl GraphicsRectangle {
    /// A filled rectangle with normalized position and size.
    pub const fn new(x: f64, y: f64, width: f64, height: f64, color: GraphicsColor) -> Self {
        Self {
            x,
            y,
            width,
            height,
            color,
        }
    }
}

/// The complete drawing state for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GraphicsScene {
    /// The color behind the rectangle.
    pub background: GraphicsColor,
    /// The solid rectangle to draw.
    pub rectangle: GraphicsRectangle,
}

impl GraphicsScene {
    /// Draws the rectangle over the supplied background.
    pub const fn new(background: GraphicsColor, rectangle: GraphicsRectangle) -> Self {
        Self {
            background,
            rectangle,
        }
    }

    /// Whether the rectangle has finite coordinates and lies inside the viewport.
    pub fn valid(&self) -> bool {
        let rect = self.rectangle;
        [rect.x, rect.y, rect.width, rect.height]
            .into_iter()
            .all(|value| (0.0..=1.0).contains(&value))
            && rect.x + rect.width <= 1.0
            && rect.y + rect.height <= 1.0
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

    use super::{GraphicsEvent, GraphicsPlacement, GraphicsRect, GraphicsScene};

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
            "rectangle": {
                "x": 0.2, "y": 0.1, "width": 0.6, "height": 0.8,
                "color": {"red": 0, "green": 255, "blue": 0}
            }
        });
        let mut drawing: GraphicsScene = serde_json::from_value(scene.clone()).unwrap();
        assert!(drawing.valid());
        assert_eq!(serde_json::to_value(drawing).unwrap(), scene);

        drawing.rectangle.width = 0.9;
        assert!(!drawing.valid());
        drawing.rectangle.width = f64::NAN;
        assert!(!drawing.valid());

        for event in [
            json!({"kind": "ready", "lease": "surface-1"}),
            json!({"kind": "failed", "error": {"code": "render_failed"}}),
        ] {
            let decoded: GraphicsEvent = serde_json::from_value(event.clone()).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), event);
        }
    }
}
