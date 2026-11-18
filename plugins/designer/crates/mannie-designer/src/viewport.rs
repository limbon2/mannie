//! The project's native viewport, its generated commands and the scene that
//! Designer draws. The desktop hosts the graphics surface for this view.

use std::sync::Arc;

use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, Handlers, InputRefusal, Interface, PluginError, Validate,
};
use mannie_plugin_sdk::services::graphics::{
    GraphicsApi, GraphicsColor, GraphicsError, GraphicsEvent, GraphicsPlacement, GraphicsRectangle,
    GraphicsScene, GraphicsView,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::plugin::{DESIGNER, Projects};

fn scope(project: String) -> GraphicsView {
    GraphicsView::new(DESIGNER, project)
}

fn validate_project(project: &str) -> Result<(), InputRefusal> {
    uuid::Uuid::try_parse(project)
        .map(|_| ())
        .map_err(|_error| InputRefusal::new("projectId"))
}

fn validate_target(project: &str, lease: &str) -> Result<(), InputRefusal> {
    validate_project(project)?;
    if lease.is_empty() || lease.len() > 128 || lease.chars().any(char::is_control) {
        return Err(InputRefusal::new("lease"));
    }

    Ok(())
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OpenViewport {
    project_id: String,
}

impl Validate for OpenViewport {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_project(&self.project_id)
    }
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlaceViewport {
    project_id: String,
    lease: String,
    placement: GraphicsPlacement,
}

impl Validate for PlaceViewport {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_target(&self.project_id, &self.lease)?;
        if !self.placement.valid() {
            return Err(InputRefusal::new("placement"));
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetViewportColor {
    project_id: String,
    lease: String,
    hue: u16,
}

impl Validate for SetViewportColor {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_target(&self.project_id, &self.lease)?;
        if self.hue > 360 {
            return Err(InputRefusal::new("hue"));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
enum ViewportError {
    #[error("the project could not be found")]
    ProjectMissing,
    #[error("this platform has no native viewport")]
    UnsupportedPlatform,
    #[error("the graphics context could not be opened")]
    GraphicsUnavailable,
    #[error("the graphics frame could not be presented")]
    RenderFailed,
    #[error("the placement is invalid")]
    InvalidPlacement,
    #[error("the viewport lease is no longer valid")]
    StaleLease,
}

impl From<GraphicsError> for ViewportError {
    fn from(error: GraphicsError) -> Self {
        match error {
            GraphicsError::UnsupportedPlatform => Self::UnsupportedPlatform,
            GraphicsError::GraphicsUnavailable => Self::GraphicsUnavailable,
            GraphicsError::RenderFailed => Self::RenderFailed,
            GraphicsError::InvalidPlacement => Self::InvalidPlacement,
            GraphicsError::StaleLease => Self::StaleLease,
        }
    }
}

impl PluginError for ViewportError {
    fn code(&self) -> &'static str {
        match self {
            Self::ProjectMissing => "designer.project_missing",
            Self::UnsupportedPlatform => "designer.unsupported_platform",
            Self::GraphicsUnavailable => "designer.graphics_unavailable",
            Self::RenderFailed => "designer.render_failed",
            Self::InvalidPlacement => "designer.invalid_placement",
            Self::StaleLease => "designer.stale_lease",
        }
    }
}

struct FollowViewportChannel;

impl ChannelSpec for FollowViewportChannel {
    const NAME: &'static str = "follow_viewport";
    const DOCS: &'static str = "Opens the project's native viewport until the channel closes.";
    type Input = OpenViewport;
    type Item = GraphicsEvent;
    type Error = ViewportError;
}

struct PlaceViewportCommand;

impl CommandSpec for PlaceViewportCommand {
    const NAME: &'static str = "place_viewport";
    const DOCS: &'static str = "Places or hides the current viewport.";
    type Input = PlaceViewport;
    type Output = ();
    type Error = ViewportError;
}

struct SetViewportColorCommand;

impl CommandSpec for SetViewportColorCommand {
    const NAME: &'static str = "set_viewport_color";
    const DOCS: &'static str = "Sets the rectangle hue in degrees on the current viewport.";
    type Input = SetViewportColor;
    type Output = ();
    type Error = ViewportError;
}

pub(crate) fn interface(base: Interface) -> Interface {
    base.channel::<FollowViewportChannel>()
        .command::<PlaceViewportCommand>()
        .command::<SetViewportColorCommand>()
}

/// Full saturation and value. Each 60-degree segment linearly changes one
/// sRGB channel between its two endpoints.
fn rectangle_color(hue: u16) -> GraphicsColor {
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

fn scene(hue: u16) -> GraphicsScene {
    let background = GraphicsColor::new(32, 34, 38);
    let rectangle = GraphicsRectangle::new(0.25, 0.30, 0.5, 0.4, rectangle_color(hue));
    GraphicsScene::new(background, rectangle)
}

pub(crate) fn handlers(
    base: Handlers,
    projects: Arc<Projects>,
    graphics: Arc<dyn GraphicsApi>,
) -> Handlers {
    let opening = Arc::clone(&graphics);
    let placing = Arc::clone(&graphics);
    base.channel::<FollowViewportChannel, _, _>(move |input| {
        let exists = projects.contains(&input.project_id);
        let graphics = Arc::clone(&opening);
        async move {
            if !exists {
                return Err(ViewportError::ProjectMissing);
            }

            Ok(graphics.open(scope(input.project_id), scene(0)).await?)
        }
    })
    .command::<PlaceViewportCommand, _, _>(move |input| {
        let graphics = Arc::clone(&placing);
        async move {
            Ok(graphics
                .place(scope(input.project_id), input.lease, input.placement)
                .await?)
        }
    })
    .command::<SetViewportColorCommand, _, _>(move |input| {
        let graphics = Arc::clone(&graphics);
        async move {
            Ok(graphics
                .draw(scope(input.project_id), input.lease, scene(input.hue))
                .await?)
        }
    })
}
