# Canvas selection

|              |                                                                                                                                                                                      |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status       | Building                                                                                                                                                                             |
| Last updated | 2026-11-20                                                                                                                                                                           |
| Part of      | [Designer](README.md)                                                                                                                                                                |
| Related      | [Canvas controls](canvas-controls.md), [Persisted document](persisted-document.md), [Scene decision](../../../docs/adr/0075-draw-bounded-graphics-scenes-in-document-coordinates.md) |

## Intent

The owner can point to the project's rectangle or use the keyboard to select it. A native outline shows which element is selected, and the webview reports the same selection to a screen reader.

## Behavior

- A newly opened canvas has no selection. A primary click inside the visible rectangle focuses the canvas and selects the rectangle. A second click on it keeps it selected. Clicking the background clears the selection.
- Tab reaches the ready canvas and shows a visible focus indicator. Enter or Space on the focused canvas selects the rectangle. Escape clears it and keeps focus on the canvas. Tab can then leave the canvas. Keys used on the hue slider or in a shell overlay operate that control.
- The outline follows the rectangle's edges and is two device pixels wide at every display scale and zoom. When a rectangle extends beyond the viewport, only the visible portions of its actual edges are outlined. Changing the window size or the rectangle's hue keeps the selection.
- The focused canvas exposes its name and keyboard instructions. A selection change announces “Rectangle”, its X and Y position, and its width and height in document units. Clearing the selection announces that nothing is selected. The Designer catalog supplies these messages and the view formats numbers for the owner's locale.
- Selection lasts for the open project viewport. Switching projects, disabling Designer or reloading drops it. Selecting and clearing leave the saved document and its revision unchanged.
- The empty project view and the loading and unsupported-platform states keep their existing actions. Selection becomes available when the viewport is visible and the document and surface are ready. If drawing fails, the canvas stops accepting selection input, shows the viewport error and offers retry. Reopening the viewport starts with the confirmed document and a clear selection.

## Fit

The webview receives input over the click-through native surface. Hit testing starts from the full viewport's client bounds in CSS pixels and uses the inverse of the scene's view transform. For this feature the view is the identity. The plugin validates the selected element against the current project and lease, then supplies its outline in a replacement scene. The host draws the outline above the fill. Graphics major 2 supplies the transform and the outline. [Graphics scene decision](../../../docs/adr/0075-draw-bounded-graphics-scenes-in-document-coordinates.md).

## Design

The generated selection command takes the project ID, surface lease and an optional element ID. Clearing sends an empty selection. Designer checks the target against the confirmed document and sends the scene on the existing lease. A replacement lease starts with a clear selection.

## Acceptance

- Headless tests cover hits inside and outside a clipped rectangle, pointer and keyboard selection, focus, accessible text, and clearing with a background click or Escape. Check that clicking and then clearing while a draw is pending leaves the latest selection on screen.
- Headless Rust tests check scene bounds, lease and project isolation, and that selection leaves the stored revision and geometry intact. Verify that a render failure reaches the view, retry restores input and a late response from a closed project cannot restore its outline.
- In the Windows viewport test, select and clear with both input methods and compare the accessible state with the native pixels. Check a straight edge for a two-device-pixel stroke across changes in scale and view transform. Run framebuffer checks offscreen where possible. A visible Windows run confirms the click passes through the native child, the focus indicator can be seen, and the composed desktop shows the same outline. [Viewport test](../tests/e2e/viewport.test.ts).

On 2026-11-20, the offscreen Windows test observed the outline within 42.1 ms of starting the pointer action, including WebDriver round trips. The visible Windows run sampled two white device pixels at the selected edge and the original fill and background after clearing.
