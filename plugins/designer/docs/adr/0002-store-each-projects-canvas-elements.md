# ADR-0002: Store each project's canvas elements

|          |                                                                                                                                                                                                                    |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Status   | Accepted                                                                                                                                                                                                           |
| Date     | 2026-11-19                                                                                                                                                                                                         |
| Deciders | Owner                                                                                                                                                                                                              |
| Related  | [Designer projects](0001-keep-the-projects-of-the-designer-plugin-in-its-own-table.md), [Canvas controls](../canvas-controls.md), [Erasure](../../../../docs/adr/0048-erase-what-plugins-delete-from-the-store.md) |

## Context

Designer projects already survive a restart. Their rectangle was sized relative to the viewport, and its hue lived in the webview. The owner wants a project to reopen with the same drawing, even in a differently sized window.

## Decision

1. Use the existing plugin store. Migration 2 adds a document revision to each project and creates `designer_elements` with a plugin-generated stable UUID, project foreign key, x, y, width, height, hue, stacking position and creation time. Index elements by project and stacking position.
2. At identity zoom one document unit occupies one CSS pixel. Geometry must be finite, both edges on each axis must lie between −32,768 and +32,768 and each dimension must be at least one unit. Save the slider's integer hue from 0 through 360 inclusive. The two red endpoints retain their distinct slider positions.
3. Revision zero means the document has never opened. On first open, insert one red rectangle at (64, 64) with width 240 and height 160 and advance the revision together. This applies to new and existing projects. An initialized document with no elements stays empty.
4. Keep previews transient. An edit targets the active lease, project, element and expected revision. Save the edit and advance its revision in one fully durable store transaction. Publish the new confirmed snapshot after the transaction succeeds. Keep the previous confirmed snapshot available when a read or write fails.
5. Future commands that delete owner content use the store's erasing durability, including project deletion that removes its elements. Turning off the plugin releases the viewport and retains the document.

## Consequences

The stored drawing is independent of window size and display scale. A project pays for initialization on its first document open. Concurrent first opens converge on the same element. A stale editor must refresh its revision before saving again.

## Alternatives considered

| Option                                  | Reason declined                                                 |
| --------------------------------------- | --------------------------------------------------------------- |
| Seed whenever a project has no elements | Recreates content after the owner deletes the last element.     |
| Save normalized viewport fractions      | Resizing the window would resize the drawing.                   |
| Save RGB alone                          | The two red slider endpoints would reopen at the same position. |

## Validation

Exercise an upgrade from migration 1, concurrent opens, bounded geometry, lease and revision isolation, and transaction rollback. In the visible Windows app, verify the saved rectangle after reload and restart and the confirmed pixels after a refused save. [Persisted document](../persisted-document.md)
