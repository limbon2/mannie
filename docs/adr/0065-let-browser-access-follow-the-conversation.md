# ADR-0065: Let browser access follow the conversation

|          |                                                                                                                                                                                                                                                |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Accepted (D58)                                                                                                                                                                                                                                 |
| Date     | 2026-11-12                                                                                                                                                                                                                                     |
| Deciders | Owner                                                                                                                                                                                                                                          |
| Related  | [ADR-0025](0025-add-side-panels-to-the-shell.md), [ADR-0044](0044-let-tool-results-carry-images-to-the-model.md), [ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md), [Browser](../../plugins/browser/docs/README.md) |

This revision replaces the text that D57 accepted, to fit the live page of [ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md) (D58).

[ADR-0067](0067-manage-browser-trust-and-credentials-in-settings.md) refines the Browser controls and Settings.

## Context

The owner wants agents to ask for browser control during takeover, use a granted site across turns, and act on several page controls from one observation. Browser tabs open in the shell's side-panel tabs, and the agent can ask for screenshots. With a live page in the pane, the owner navigates, types and clicks in the page itself during takeover, so the browser sees the owner's actions only as their results: new documents, new origins and changed pages. The owner decided that after takeover the agent gets no screenshot of a document that the owner used until the tab navigates, and that the page's context menu and browser keys work in takeover without save, print, downloads and DevTools (D58).

## Decision

1. When the owner has the browser, an otherwise valid agent Browser tool call asks the owner for control in the conversation. The agent can also use `browser_request_control` to ask directly. Approval authorizes one transfer from the current owner control state. Full access does not grant a handoff. The browser service checks the conversation, session and control generation before transferring, and invalidates existing input references. The handoff result asks the agent to prepare the original action under site policy if it still needs to run.
2. The owner can approve an exact HTTP or HTTPS origin for the conversation. A stored grant names the Browser plugin, origin and conversation. The owner can revoke it in the conversation permissions. Ending a browser session clears its folder and credentials, and a conversation site grant remains available when a new session starts. Other conversations ask for their own grant. A new origin still requires approval before a top-level request, including on redirects and popups.
3. In takeover the owner navigates natively, so each tab keeps an ordered list of the origins that were approved for it, and the commit check accepts any of them. An approval that the owner gives for an owner navigation is not bound to the tab's revision. A page's first top-level commit after a popup replaced its contents is checked like any other, and its first document request is always treated as top-level.
4. During takeover the page's context menu keeps editing, navigation and link items, and the browser keys work, with save, print, downloads and DevTools removed. JavaScript dialogs are answered by the owner in takeover and dismissed under agent control. When the owner resumes agent control, the current document of every tab that the owner used is withheld from model screenshots until that tab navigates, as protected documents are ([ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md), decision 10). Text observations of it are still redacted.
5. The Browser plugin uses the shell's side-panel tabs ([ADR-0025](0025-add-side-panels-to-the-shell.md)). A panel shows one tab of one conversation. The SDK can open a panel in the background, replace a launcher panel in place and update the label of an open panel. A background open leaves a full strip intact. Watching a tab leaves the agent's selected tab alone. Closing a panel hides its page. The Browser toolbar can close the tab itself. The owner can return to tabs whose panels are closed or evicted. The existing panel limit applies.
6. `browser_observe` can include an image of the page as a tool image ([ADR-0044](0044-let-tool-results-carry-images-to-the-model.md)), captured at the pane's size without a clip. The result reports the image dimensions, and point coordinates that the agent gives are pixels of the image, which the service divides by the device pixel ratio. Withheld documents keep their text observation and get no image. The agent outline reports the checked state of checkboxes when the engine supplies it.
7. `browser_interact` accepts an ordered, bounded batch of element clicks from one observation. The journal records the batch before dispatch. The service checks control, origin, tab, document and the next live target before each click. A named checkbox rebuilt in the same document can be matched by its accessible name and checked state when the match is unique. The batch stops on a failed check, navigation, cancellation or an uncertain dispatch. The result reports progress and identifies the first unfinished action. The agent can observe once after the batch to verify the result or recover.

8. `browser_interact` can drag from one element or point to another, from one observation, under the same site grant, journal entry and target checks as a click (D59). The driver turns on drag interception: an HTML drag hands its data to the driver instead of starting the operating system's drag, and the driver plays enter, drag-over steps and the drop with `DevTools` drag events; a pointer-based library sees the press, the moves and the release. List items, rows, tree items and cells get element references, named by their text when they have no name of their own, so an agent can name what it drags. `press` and `scroll` act on the page as it is and need no new observation, so Enter can follow a fill.

These decisions refine [ADR-0064](0064-give-agents-an-isolated-browser-beside-the-conversation.md), decisions 5 through 10. Credential protections in its decision 10 still apply to text, images and persisted output.

## Consequences

- A conversation can return to a granted site in a later turn or browser session. Revoking the grant makes later agent access ask again.
- The owner's takeover is ordinary browsing. What the owner typed or saw in a document stays out of model images until the tab leaves it.
- A page can rebuild its controls between clicks. An ambiguous or changed target interrupts the batch and leaves earlier clicks applied.
- The browser's tab count can exceed the shell's panel limit. The Browser control can reopen a view of a live tab.

## Alternatives considered

| Option                                     | Why not chosen                                                                                                        |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| Keep run-only approval                     | The owner must approve an origin again at the start of each turn.                                                     |
| Treat site approval as control             | An earlier site grant could transfer control while the owner is using the browser.                                    |
| Screenshot owner-used documents            | Text the owner typed in takeover, a password among it, could reach the model in an image.                             |
| Turn off the context menu and browser keys | Takeover would feel unlike a browser. The filtered set keeps editing and navigation and removes what leaves the page. |
| Ask the model to verify every click        | A todo list requires a separate model round for each checkbox.                                                        |
| Batch arbitrary coordinates or keys        | Page changes can move a coordinate target or put focus on a different control mid-batch.                              |

## Validation

Extend [V13](../roadmap/milestones.md#4-validations) with a desktop run on Windows that approves a handoff, opens two tabs in the shell, returns to an approved origin across runs, denies a different origin, sends a screenshot to the model, withholds the screenshot of a document that the owner typed in until the tab navigates, and applies one batch to a todo list that rebuilds after each click. Test a partially completed batch, the filtered context menu and browser keys, and keep the protected sign-in fixture.
