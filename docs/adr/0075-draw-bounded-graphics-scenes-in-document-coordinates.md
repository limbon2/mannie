# ADR-0075: Draw bounded graphics scenes in document coordinates

|          |                                                                                                                                                                                                               |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D70)                                                                                                                                                                                                |
| Date     | 2026-11-20                                                                                                                                                                                                    |
| Deciders | Owner                                                                                                                                                                                                         |
| Related  | [Native graphics surfaces](0073-host-native-graphics-surfaces-for-desktop-plugins.md), [Plugin SDK](0034-build-plugins-on-a-plugin-sdk.md), [Canvas controls](../../plugins/designer/docs/canvas-controls.md) |

## Context

Designer saves rectangle geometry in document units. Graphics major 1 accepts one filled rectangle clipped to the viewport. Selecting a rectangle needs a native outline that follows its original edges, including when an edge falls outside the visible area. The child window paints above the webview, which owns pointer and keyboard input.

## Decision

1. Replace the unreleased `graphics` major 1 scene with major 2. This supersedes the major in decision 1 and the scene in decision 5 of ADR-0073. Designer is the only consumer. Update the SDK, desktop provider and Designer manifest together. The host refuses a service major it cannot provide. The stored document and its revision are independent of the service version.
2. A scene is a complete snapshot with an opaque background, a view, an ordered list of shapes and an ordered list of overlays. Paint the background, then the shapes in order, then the overlays in order. The first shape is a solid rectangle and the first overlay is a rectangle outline with an opaque color and a two-device-pixel stroke. Add other overlay kinds when a feature draws them. Element IDs and selection state belong to the plugin.
3. The view gives the document point at the top left of the full viewport and a zoom. On each axis, the CSS pixel coordinate is `(document - origin) * zoom`. Hit testing uses its inverse. At identity, the origin is zero and the zoom is one. The host applies display scale once, strokes the transformed edges in device pixels and clips the result to the visible client area. A clipped edge of the viewport must never become an edge of the outline.
4. Require finite numbers. Document edges and each coordinate of the view origin are in the inclusive range −32,768 to +32,768. Rectangles have positive dimensions. Zoom is between 0.05 and 64 inclusive. A scene may have empty lists, at most 1,024 shapes and 1,024 overlays, and at most 256 KiB in compact JSON encoding. These limits apply independently. The SDK exposes the limits and validation to both caller and host. The plugin validates its inputs and resulting scene. The host validates on open and on every replacement before retaining or scheduling the scene. A rejected scene leaves the previous scene intact.
5. Opening and drawing use the existing scoped surface lease. Keep at most one draw in flight and coalesce pending previews to the newest complete scene for that lease. When a hidden surface becomes visible it presents the current scene. Window resources and input ownership follow ADR-0073.

## Consequences

Resizing reveals or clips the same document geometry. A selection outline has a predictable physical thickness at every zoom and display scale. A major mismatch prevents a plugin from binding to an incompatible host. The list and byte limits bound the work per replacement and may limit the number of elements a later feature can show in one frame.

## Alternatives considered

| Option                                           | Reason declined                                                                                            |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| Draw an outline in the webview                   | The native child window covers it.                                                                         |
| Add a selection flag to the normalized rectangle | The host would acquire editor state, and clipping the rectangle before stroking it would draw false edges. |
| Send individual overlay or shape edits           | Dropped or late edits would require additional state to reconstruct the scene for a lease.                 |

## Validation

Exercise empty scenes, order, both sides of each bound, mismatched majors and stale leases in headless tests. Rasterize a partly cropped outline at several zooms and display scales. Sample across a true edge and across the crop to verify the two-device-pixel stroke and the absence of a false edge. Check pointer and keyboard selection against the native Windows framebuffer offscreen, then verify the composited outline in the visible Windows app. [Canvas selection](../../plugins/designer/docs/selection.md)
