# Persisted document

|              |                                                                                                                                     |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                            |
| Last updated | 2026-11-19                                                                                                                          |
| Part of      | [Designer](README.md)                                                                                                               |
| Related      | [Canvas controls](canvas-controls.md), [Storage decision](adr/0002-store-each-projects-canvas-elements.md), [Viewport](viewport.md) |

## Intent

The owner can return to a Designer project after a restart and continue with its saved rectangle. The drawing keeps its position and size when the window changes.

## Behavior

The first time a project opens its drawing it receives one red rectangle. The Rectangle color slider previews a hue during a drag and saves the final value on release. The keyboard edits the same fill. Reopening either of two projects restores that project's exact hue, including 360°. Resizing reveals or clips the drawing.

A failed save restores the confirmed hue and rectangle. The error and retry action appear beside the slider. A read failure keeps the project's last confirmed rectangle visible when one is available. On the first read a loading indicator gives way to a retry action if the document cannot be opened. Linux keeps the platform availability message and project management.

## Fit

Designer owns the elements, migrations and plugin commands. The existing graphics service paints one normalized rectangle. Designer clips its document geometry to the full logical viewport before passing the scene to the host. The host applies the display scale and visible crop. [Native graphics service](../../../docs/adr/0073-host-native-graphics-surfaces-for-desktop-plugins.md)

## Design

The initial geometry, project revision and write boundary follow [Designer ADR-0002](adr/0002-store-each-projects-canvas-elements.md). Each edit addresses the active project and viewport lease. The plugin supplies confirmed snapshots to the webview and accepts transient hue previews between commits. The webview coalesces previews, keeps each completed edit in order and reads a fresh snapshot on recovery. The plugin catalog supplies document load and save errors.

## Acceptance

- Upgrade a project created under migration 1. Concurrent opens seed exactly one rectangle. Check stable identity and geometry after reopening it.
- In the visible Windows app, give two projects different hues. Reload the page and restart the process. Compare each slider and the rectangle's interior and exterior pixels before and after resizing.
- Force a failed read and a failed write. Check the last confirmed value, use Retry, and reopen the project to verify the saved result. Check that changing projects and closing a viewport discard late replies.
- Click the middle of the native rectangle and check that the webview placeholder receives focus. Measure the time from a slider movement to the changed pixel on the composed desktop.

The Windows [viewport test](../tests/e2e/viewport.test.ts) runs with the validation window offscreen by default. It checks the slider, saved rows and native framebuffer. A visible run with `MANNIE_E2E_SHOW_WINDOW=1` also samples the composed desktop.

On 2026-11-19 the offscreen Windows run observed the changed framebuffer within 23.4 ms of dispatching the slider movement, including WebDriver round trips.
