//! The project's native viewport, its generated commands and the scene that
//! Designer draws. The desktop hosts the graphics surface for this view.

use std::sync::Arc;

use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, Handlers, InputRefusal, Interface, PluginError, Validate,
};
use mannie_plugin_sdk::services::graphics::{
    GraphicsApi, GraphicsError, GraphicsEvent, GraphicsPlacement,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::document::DocumentSnapshot;
use crate::plugin::Projects;

mod session;
use session::Viewports;

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

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ViewportTarget {
    project_id: String,
    lease: String,
}

impl Validate for ViewportTarget {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_target(&self.project_id, &self.lease)
    }
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

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommitViewportColor {
    project_id: String,
    lease: String,
    element_id: String,
    revision: u32,
    hue: u16,
}

impl Validate for CommitViewportColor {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_target(&self.project_id, &self.lease)?;
        uuid::Uuid::try_parse(&self.element_id).map_err(|_error| InputRefusal::new("elementId"))?;

        if self.hue > 360 {
            return Err(InputRefusal::new("hue"));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, thiserror::Error)]
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
    #[error("the saved document changed")]
    StaleRevision,
    #[error("the document could not be read or saved")]
    StorageFailed { confirmed: Option<DocumentSnapshot> },
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
            Self::StaleRevision => "designer.stale_revision",
            Self::StorageFailed { .. } => "designer.storage_failed",
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
    const DOCS: &'static str = "Previews the rectangle hue on the current viewport.";
    type Input = SetViewportColor;
    type Output = ();
    type Error = ViewportError;
}

struct ReadViewportDocumentCommand;

impl CommandSpec for ReadViewportDocumentCommand {
    const NAME: &'static str = "read_viewport_document";
    const DOCS: &'static str = "Reads or initializes the current viewport's saved drawing.";
    type Input = ViewportTarget;
    type Output = DocumentSnapshot;
    type Error = ViewportError;
}

struct CommitViewportColorCommand;

impl CommandSpec for CommitViewportColorCommand {
    const NAME: &'static str = "commit_viewport_color";
    const DOCS: &'static str =
        "Saves one completed fill edit against its confirmed document revision.";
    type Input = CommitViewportColor;
    type Output = DocumentSnapshot;
    type Error = ViewportError;
}

pub(crate) fn interface(base: Interface) -> Interface {
    base.channel::<FollowViewportChannel>()
        .command::<PlaceViewportCommand>()
        .command::<SetViewportColorCommand>()
        .command::<ReadViewportDocumentCommand>()
        .command::<CommitViewportColorCommand>()
}

pub(crate) fn handlers(
    base: Handlers,
    projects: Arc<Projects>,
    graphics: Arc<dyn GraphicsApi>,
) -> Handlers {
    let viewports = Arc::new(Viewports::new(projects, graphics));
    let opening = Arc::clone(&viewports);
    let reading = Arc::clone(&viewports);
    let placing = Arc::clone(&viewports);
    let drawing = Arc::clone(&viewports);
    base.channel::<FollowViewportChannel, _, _>(move |input| {
        let viewports = Arc::clone(&opening);
        async move { viewports.open(input.project_id).await }
    })
    .command::<ReadViewportDocumentCommand, _, _>(move |input| {
        let viewports = Arc::clone(&reading);
        async move { viewports.read(input).await }
    })
    .command::<PlaceViewportCommand, _, _>(move |input| {
        let viewports = Arc::clone(&placing);
        async move { viewports.place(input).await }
    })
    .command::<SetViewportColorCommand, _, _>(move |input| {
        let viewports = Arc::clone(&drawing);
        async move { viewports.preview(input).await }
    })
    .command::<CommitViewportColorCommand, _, _>(move |input| {
        let viewports = Arc::clone(&viewports);
        async move { viewports.commit(input).await }
    })
}
