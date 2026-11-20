//! Device-pixel placement and the offset of the native crop within a frame.

use mannie_plugin_sdk::services::graphics::{
    GraphicsPlacement, GraphicsRect, GraphicsRectangle, GraphicsTransform,
};
use skia_safe::Rect;
use windows::Win32::Foundation::RECT;

#[derive(Debug)]
pub(super) struct DrawingArea {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) width: i32,
    pub(super) height: i32,
    origin_x: f64,
    origin_y: f64,
    scale: f64,
}

impl DrawingArea {
    pub(super) fn from_placement(placement: &GraphicsPlacement, client: RECT) -> Option<Self> {
        if !placement.visible || !placement.valid() {
            return None;
        }

        let full = physical(placement.bounds, placement.scale_factor);
        let clip = physical(placement.clip, placement.scale_factor);
        let left = full.left.max(clip.left).max(client.left);
        let top = full.top.max(clip.top).max(client.top);
        let right = full.right.min(clip.right).min(client.right);
        let bottom = full.bottom.min(clip.bottom).min(client.bottom);

        if left >= right || top >= bottom {
            return None;
        }

        Some(Self {
            left,
            top,
            width: right - left,
            height: bottom - top,
            origin_x: placement.bounds.x * placement.scale_factor - f64::from(left),
            origin_y: placement.bounds.y * placement.scale_factor - f64::from(top),
            scale: placement.scale_factor,
        })
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "bounded physical coordinates fit Skia's f32 canvas"
    )]
    pub(super) fn rectangle(&self, rectangle: GraphicsRectangle, view: GraphicsTransform) -> Rect {
        let scale = view.zoom * self.scale;
        let project_x = |x| (self.origin_x + (x - view.origin_x) * scale) as f32;
        let project_y = |y| (self.origin_y + (y - view.origin_y) * scale) as f32;
        Rect::new(
            project_x(rectangle.x),
            project_y(rectangle.y),
            project_x(rectangle.x + rectangle.width),
            project_y(rectangle.y + rectangle.height),
        )
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "validated CSS coordinates and scale fit i32"
)]
fn physical(rect: GraphicsRect, scale: f64) -> RECT {
    RECT {
        left: (rect.x * scale).round() as i32,
        top: (rect.y * scale).round() as i32,
        right: ((rect.x + rect.width) * scale).round() as i32,
        bottom: ((rect.y + rect.height) * scale).round() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mannie_plugin_sdk::services::graphics::GraphicsColor;

    #[test]
    fn clipping_preserves_the_full_viewports_scene_coordinates() {
        let bounds = GraphicsRect::new(-10.0, 2.0, 100.0, 80.0);
        let clip = GraphicsRect::new(-5.0, 12.0, 85.0, 40.0);
        let placement = GraphicsPlacement::new(bounds, clip, 2.0, true);
        let client = RECT {
            left: 0,
            top: 0,
            right: 400,
            bottom: 500,
        };
        let area = DrawingArea::from_placement(&placement, client).unwrap();
        assert_eq!(
            (area.left, area.top, area.width, area.height),
            (0, 24, 160, 80)
        );
        let rect = GraphicsRectangle::new(25.0, 20.0, 50.0, 40.0, GraphicsColor::new(0, 0, 0));
        assert_eq!(
            area.rectangle(rect, GraphicsTransform::IDENTITY),
            Rect::from_xywh(30.0, 20.0, 100.0, 80.0)
        );
    }
}
