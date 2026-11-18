//! Skia wraps the default, double-buffered framebuffer of the native child.

use std::num::NonZeroU32;

use anyhow::{Context as _, Result};
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::{Display, DisplayApiPreference, GlDisplay as _};
use glutin::prelude::{GlSurface as _, NotCurrentGlContext as _, PossiblyCurrentGlContext as _};
use glutin::surface::{Surface as GlSurface, SurfaceAttributesBuilder, WindowSurface};
use mannie_plugin_sdk::services::graphics::{GraphicsColor, GraphicsError, GraphicsScene};
use raw_window_handle::{Win32WindowHandle, WindowsDisplayHandle};
use skia_safe::gpu::{self, DirectContext, SurfaceOrigin, backend_render_targets, gl};
use skia_safe::{Color, ColorType, Paint, Surface};

use super::geometry::DrawingArea;

pub(super) struct Renderer {
    target: Surface,
    skia: DirectContext,
    context: PossiblyCurrentContext,
    surface: GlSurface<WindowSurface>,
    samples: usize,
    stencil: usize,
    #[cfg(feature = "validation")]
    pub(super) pixels: Option<super::validation::FramePixels>,
}

impl Renderer {
    pub(super) fn new(window: Win32WindowHandle) -> Result<Self> {
        let raw = window.into();
        // SAFETY: the calling thread owns the live child and its HINSTANCE,
        // and its owner retains it beyond the lifetime of every glutin object.
        let display = unsafe {
            Display::new(
                WindowsDisplayHandle::new().into(),
                DisplayApiPreference::Wgl(Some(raw)),
            )
        }?;

        let template = ConfigTemplateBuilder::new()
            .compatible_with_native_window(raw)
            .with_alpha_size(8)
            .with_stencil_size(8)
            .build();
        // SAFETY: template contains only the live child passed to this display.
        let config = unsafe { display.find_configs(template) }?
            .min_by_key(GlConfig::num_samples)
            .context("no WGL window pixel format")?;

        let attributes = ContextAttributesBuilder::new().build(Some(raw));
        // SAFETY: config came from this display and raw names its live child.
        let context = unsafe { display.create_context(&config, &attributes) }?;
        let attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            raw,
            NonZeroU32::MIN,
            NonZeroU32::MIN,
        );
        // SAFETY: the hidden 1x1 child owns this pixel format and outlives the surface.
        let surface = unsafe { display.create_window_surface(&config, &attributes) }?;
        let context = context.make_current(&surface)?;
        let interface = gl::Interface::new_load_with_cstr(|name| {
            if name == c"eglGetCurrentDisplay" {
                return std::ptr::null();
            }

            display.get_proc_address(name)
        })
        .context("Skia could not load WGL's OpenGL entry points")?;

        let mut skia = gpu::direct_contexts::make_gl(interface, None)
            .context("Skia could not initialize its OpenGL context")?;

        let samples = usize::from(config.num_samples());
        let stencil = usize::from(config.stencil_size());
        let target = framebuffer(&mut skia, (1, 1), samples, stencil)?;
        Ok(Self {
            target,
            skia,
            surface,
            context,
            samples,
            stencil,
            #[cfg(feature = "validation")]
            pixels: None,
        })
    }

    pub(super) fn draw(
        &mut self,
        area: &DrawingArea,
        scene: &GraphicsScene,
    ) -> Result<(), GraphicsError> {
        self.present(area, scene).map_err(|error| {
            tracing::warn!(%error, "could not present the native graphics frame");
            GraphicsError::RenderFailed
        })
    }

    fn present(&mut self, area: &DrawingArea, scene: &GraphicsScene) -> Result<()> {
        self.context.make_current(&self.surface)?;

        let size = (area.width, area.height);

        if size != (self.target.width(), self.target.height()) {
            // WGL follows the HWND's client size. Only Skia's wrapper retains
            // its old dimensions, and must be recreated after SetWindowPos.
            self.target = framebuffer(&mut self.skia, size, self.samples, self.stencil)?;
        }

        let canvas = self.target.canvas();
        canvas.clear(color(scene.background));

        let mut paint = Paint::default();
        paint.set_color(color(scene.rectangle.color));
        canvas.draw_rect(area.rectangle(scene.rectangle), &paint);
        self.skia
            .flush_and_submit()
            .context("Skia failed to submit the frame")?;

        #[cfg(feature = "validation")]
        let pixels = super::validation::FramePixels::read(&mut self.target);
        self.surface.swap_buffers(&self.context)?;
        #[cfg(feature = "validation")]
        {
            self.pixels = Some(pixels);
        }
        Ok(())
    }
}

fn framebuffer(
    context: &mut DirectContext,
    size: (i32, i32),
    samples: usize,
    stencil: usize,
) -> Result<Surface> {
    // WGL renders directly to the child window's default framebuffer (ID 0).
    let info = gl::FramebufferInfo {
        fboid: 0,
        format: gl::Format::RGBA8.into(),
        ..Default::default()
    };
    let backend = backend_render_targets::make_gl(size, samples, stencil, info);
    gpu::surfaces::wrap_backend_render_target(
        context,
        &backend,
        SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        None,
        None,
    )
    .context("Skia could not wrap the window framebuffer")
}

fn color(value: GraphicsColor) -> Color {
    Color::from_rgb(value.red, value.green, value.blue)
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // Skia can delete driver resources only with the right context current.
        // After context loss it must abandon its references without calling GL.
        if self.context.make_current(&self.surface).is_ok() {
            self.skia.release_resources_and_abandon();
            if let Err(error) = self.context.make_not_current_in_place() {
                tracing::debug!(%error, "could not detach the retired graphics context");
            }
        } else {
            self.skia.abandon();
        }
    }
}
