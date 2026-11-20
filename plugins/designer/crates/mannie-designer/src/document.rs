//! The project's saved rectangle, in document units (Designer ADR-0002).

use mannie_plugin_sdk::services::store::StoreError;
use serde::{Deserialize, Serialize};
use specta::Type;
use uuid::Uuid;

const DOCUMENT_LIMIT: f64 = 32_768.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RectangleGeometry {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl RectangleGeometry {
    pub(crate) fn valid(self) -> bool {
        let bounds = -DOCUMENT_LIMIT..=DOCUMENT_LIMIT;
        let edges = [self.x, self.y, self.x + self.width, self.y + self.height];
        self.width >= 1.0 && self.height >= 1.0 && edges.iter().all(|edge| bounds.contains(edge))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum DocumentEdit {
    Hue(u16),
    Geometry(RectangleGeometry),
}

impl DocumentEdit {
    pub(crate) fn apply(self, element: &mut DocumentElement) {
        match self {
            Self::Hue(hue) => element.hue = hue,
            Self::Geometry(geometry) => {
                element.x = geometry.x;
                element.y = geometry.y;
                element.width = geometry.width;
                element.height = geometry.height;
            }
        }
    }
}

/// Geometry and hue that the plugin confirmed for one element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocumentElement {
    pub(crate) id: String,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) hue: u16,
}

impl DocumentElement {
    pub(crate) fn seed() -> Self {
        Self {
            id: Uuid::now_v7().to_string(),
            x: 64.0,
            y: 64.0,
            width: 240.0,
            height: 160.0,
            hue: 0,
        }
    }

    pub(crate) fn geometry(&self) -> RectangleGeometry {
        RectangleGeometry {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }

    pub(crate) fn valid(&self) -> bool {
        self.geometry().valid() && self.hue <= 360 && Uuid::try_parse(&self.id).is_ok()
    }
}

/// The revision and the element come from the same committed transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocumentSnapshot {
    pub(crate) revision: u32,
    pub(crate) element: DocumentElement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentFailure {
    ProjectMissing,
    StaleRevision,
    StorageFailed,
}

impl From<StoreError> for DocumentFailure {
    fn from(error: StoreError) -> Self {
        tracing::warn!(code = error.code(), "the Designer document store failed");
        Self::StorageFailed
    }
}
