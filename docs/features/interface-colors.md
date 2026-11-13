# Interface colors

|              |                                                                                                                                                                                          |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Building                                                                                                                                                                                 |
| Milestone    | Unscheduled                                                                                                                                                                              |
| Last updated | 2026-11-13                                                                                                                                                                               |
| Related      | [ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md), [ADR-0019](../adr/0019-keep-visible-text-in-message-catalogs.md), [ADR-0020](../adr/0020-complete-the-ui-shell-slots.md) |

## Intent

The owner wants to choose a primary color and a surface color for Mannie's window. Two rows of swatches make it possible to try a combination and see it applied as soon as a color is selected.

## Behavior

- Under Settings → Interface, Primary offers Default, Blue, Teal, Green, Amber, Violet, and Rose. The choice colors primary actions, links, keyboard focus rings, and selection highlights.
- Surface offers Neutral, Slate, Stone, and Olive. The choice colors the window, sidebar, cards, popovers, and neutral hover states. Tooltips and selection highlights combine both colors.
- The two choices are independent and apply in Light, Dark, and Same as the system modes. Default primary and Neutral surface restore Mannie's neutral palette.
- A checkmark shows the selected swatch in each row. A swatch shows its name when hovered or focused with the keyboard. Tab moves between groups and the arrow keys choose a color. Rows wrap when the window is narrow.
- Mannie remembers both choices on this device and applies them before the first paint. An absent or invalid stored value uses that group's default. A choice that cannot be saved still applies for the current window.

## Fit

The picker uses the shell's existing Interface settings section and the `shell_` message catalog prefix. Its values are UI preferences in webview storage, as permitted by [ADR-0004](../adr/0004-keep-system-access-in-the-rust-core.md).

## Design

The startup theme script restores each stored choice through a fixed list of palette IDs. The selected IDs set classes on the document root, where the stylesheet maps them to Mannie's theme tokens. The same classes let an open terminal pick up surface changes through its existing theme observer. Each palette has light and dark colors. Destructive actions use the status red, with a contrasting label on hover.

## Acceptance

- In the desktop shell, mouse and keyboard selections update the app immediately. Choosing a default restores that group's colors. Both choices survive mode changes, a page reload, and an app restart. An invalid stored value falls back independently of the other choice.
- The picker works at narrow widths and in the pseudo-locale. Axe checks the picker in Light and Dark. The shell tests measure contrast for shared focus indicators, tooltips, and hovered destructive buttons, along with primary actions, links, and muted text.
