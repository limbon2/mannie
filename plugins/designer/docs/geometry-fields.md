# Geometry fields

|              |                                                                                                                      |
| ------------ | -------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                             |
| Last updated | 2026-11-20                                                                                                           |
| Part of      | [Designer](README.md)                                                                                                |
| Related      | [Canvas controls](canvas-controls.md), [Canvas selection](selection.md), [Persisted document](persisted-document.md) |

## Intent

The owner can position and size the selected rectangle precisely with the keyboard. Typing a value shows where the rectangle will appear before the owner saves the edit.

## Behavior

X, Y, Width and Height appear below the ready canvas. Selecting the rectangle fills the fields with its geometry in document units. Tab from the canvas reaches the fields in order. With the selection cleared, the fields are empty and disabled.

A complete, valid value previews the rectangle and its outline on the native canvas. Enter, Tab or clicking away saves a changed value once. Escape in the field restores its value and the saved rectangle, keeping the selection. Changing projects, losing the viewport or hiding it cancels an unfinished preview.

The owner can type negative positions and fractional values. An incomplete or invalid value remains editable and restores the confirmed geometry on the canvas. Trying to save it shows a message associated with the field. A failed save restores the confirmed geometry and offers the document's retry action. A successful save updates the selection announcement, and reopening the project restores the saved rectangle.

## Fit

Designer uses its existing document coordinates, geometry limits and durable revision check. The desktop host draws the rectangle and outline from the same scene. [Designer storage decision](adr/0002-store-each-projects-canvas-elements.md), [graphics scene decision](../../../docs/adr/0075-draw-bounded-graphics-scenes-in-document-coordinates.md).

## Design

The plugin's geometry preview and commit commands take the project, viewport lease, element, expected revision and four geometry values. Designer validates the complete rectangle before drawing it. A preview lives on its lease. A commit saves the four coordinates and advances the document revision together. The webview keeps the text being edited, combines each completed field change with the latest confirmed document, and coalesces previews while a draw is pending.

## Acceptance

- Type into all four fields. Compare the native fill and outline during a preview with the saved row, then check one revision increase after accepting the change. Verify Escape, incomplete text, an out-of-bounds edge and a failed write.
- Tab quickly between fields and interleave color and geometry edits. Verify the final rectangle, selection announcement and pointer hits. Check that an interruption clears the preview and that an old lease cannot change another project.
- In the Windows [viewport test](../tests/e2e/viewport.test.ts), reopen projects with different geometry after a page reload and a process restart and compare the stored rows with the native pixels.
