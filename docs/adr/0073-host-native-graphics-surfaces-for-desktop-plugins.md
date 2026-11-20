# ADR-0073: Host native graphics surfaces for desktop plugins

|          |                                                                                                                   |
| -------- | ----------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D68)                                                                                                    |
| Date     | 2026-11-18                                                                                                        |
| Deciders | Owner                                                                                                             |
| Related  | [Plugin SDK](0034-build-plugins-on-a-plugin-sdk.md), [Designer viewport](../../plugins/designer/docs/viewport.md) |

Later changes: [ADR-0075](0075-draw-bounded-graphics-scenes-in-document-coordinates.md) replaces the service major in decision 1 and the drawing contract in decision 5.

## Context

Designer needs to present live 2D graphics inside its main view. Its controls and the shell share the desktop webview. A native child window has its own stacking and focus rules, and can outlive the page that requested it.

## Decision

1. Add the manifest-declared desktop SDK service `graphics`, major 1. The desktop host binds it to the contributor that requested it. The plugin supplies a view, project and bounded drawing data. Its generated IPC commands carry state and placement. Native window handles, graphics calls and frame buffers stay in the desktop host.
2. Opening a surface leases it to the status stream for its view and project in the main window. A host-issued token identifies that opening. Replacing or dropping the stream, reloading the page, stopping the plugin and shutting down the app release its resources. A command for an expired lease cannot move or redraw another surface.
3. The first backend uses a Windows child window, glutin for a WGL context and presentation, and Skia through `skia-safe` with OpenGL support for drawing into the window's framebuffer. The desktop host owns these resources on the window thread. Only its Windows graphics integration module may use the additional `unsafe` calls required by these APIs. Each block documents its safety preconditions. Verified 2026-11-18. [Glutin display API](https://docs.rs/glutin/0.32.3/glutin/display/trait.GlDisplay.html), [Rust Skia OpenGL example](https://github.com/rust-skia/rust-skia/blob/master/skia-safe/examples/gl-window/main.rs).
4. The surface accepts no input. The webview reports the bounds of the full viewport, its visible portion and the display scale. The host validates the measurements and clips to its own client area. A covering overlay hides the surface. Rendering resumes with the current scene when visible again.
5. The first drawing contract contains an opaque background and one normalized, solid-color rectangle. Designer owns the geometry and colors. Unsupported systems report availability when a view asks for graphics. The rest of the plugin can still start.

## Consequences

The webview keeps keyboard focus and sends small state changes as its controls move. The Windows desktop build gains a native graphics dependency and needs a driver with a compatible OpenGL context. A failed initialization or presentation is reported to the view.

## Alternatives considered

| Option                         | Reason declined                                                                                     |
| ------------------------------ | --------------------------------------------------------------------------------------------------- |
| Draw in a webview canvas       | The owner requires native OpenGL presentation.                                                      |
| Put native handles in Designer | Platform resources would cross the SDK boundary and their lifetime would depend on a single plugin. |

## Validation

Check project and lease isolation, clipping, repaint, error recovery and teardown. Exercise the slider through the real Windows webview and check the colors presented by the native window. Check the unsupported state on Linux. [Designer viewport](../../plugins/designer/docs/viewport.md).
