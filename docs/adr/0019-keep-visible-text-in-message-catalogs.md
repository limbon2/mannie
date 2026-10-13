# ADR-0019: Keep all visible text in message catalogs, with English first

|          |                                                                                                                                                                                                                                                                |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status   | Proposed                                                                                                                                                                                                                                                       |
| Date     | 2026-10-13                                                                                                                                                                                                                                                     |
| Deciders | Owner                                                                                                                                                                                                                                                          |
| Related  | [ADR-0005](0005-build-the-ui-with-sveltekit-as-a-static-spa.md), [ADR-0007](0007-generate-ipc-bindings-from-rust.md), [Desktop app](../architecture/desktop-app.md#10-languages), [Extension points](../architecture/extension-points.md#311-message-catalogs) |

## Context

Verified 2026-10-13:

- The owner wants English first, and a way to add languages later without pain and without restructuring the project ([decision D10](../roadmap/milestones.md#5-decisions)).
- Text written directly into components has to be found and extracted from every component when the first translation arrives. That is the restructuring the owner wants to avoid.
- Paraglide JS 2.27.0 (MIT) compiles message catalogs into typed message functions, so a missing or misspelled key fails the build. The Svelte CLI sets it up with `sv add paraglide`. It supports custom client strategies that read the locale from application settings, and `setLocale` reloads the page by default.
- Reloading the webview is harmless in Mannie, because runs live in the Rust core ([ADR-0012](0012-run-agents-as-durable-actors.md)).

## Decision

1. All user-visible text in the desktop UI comes from message catalogs in `apps/desktop/messages/`, one file per locale, compiled by Paraglide JS into typed message functions. English (`en`) is the base locale and, for now, the only one that ships.
2. Components contain no literal user-facing text. They call message functions. Each feature owns its messages, under a key prefix of its own ([extension points](../architecture/extension-points.md#311-message-catalogs)).
3. The core never sends display text to the UI. Errors and statuses cross IPC as stable codes with arguments ([ADR-0007](0007-generate-ipc-bindings-from-rust.md)), and the UI turns them into messages. Content is shown as it is: model output, file contents, command output and the owner's own text are never translated.
4. Dates, times, numbers, sizes and durations are formatted with `Intl` in the current locale, never assembled by hand.
5. The locale is a setting in the core. Its default is the operating system's language when a catalog exists for it, and English otherwise. The UI reads it at startup through a custom Paraglide strategy, and changing it reloads the webview.
6. A pseudo-locale is generated from the English catalog for development and tests. It marks and lengthens every message, so the end-to-end tests can flag visible text that bypasses the catalog and layouts that break under longer translations.
7. Mannie's own components use direction-neutral (logical) styling where direction matters, so that a right-to-left language does not mean restyling them. A right-to-left language still needs its own check of the shadcn-svelte components when it is requested.
8. Adding a language means adding its catalog file and registering the locale in the Paraglide project settings. No component or Rust code changes. A check reports keys that a locale is missing.
9. Logs, internal error details, text written for the model (system prompts and tool descriptions), the worker's command-line output and the documentation stay in English.

## Consequences

- Positive: a new language is a data change. Text that bypasses the catalog is caught early, and layouts are tested against longer text from the start.
- Negative: every piece of visible text needs a key, and changing the language reloads the webview.
- Risks: hard-coded text can still slip in where the tests do not look (risk R17). The pseudo-locale run in the end-to-end tests limits this.

## Alternatives considered

| Option                                 | Why not chosen                                                                                          |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| English literals now, extraction later | Every component would change when the first translation arrives, which is what the owner wants to avoid |
| svelte-i18n                            | Runtime lookup by string key, with no build-time check of keys or arguments                             |
| i18next                                | A large runtime, and untyped keys unless extra tooling generates types                                  |
| Fluent (`@fluent/bundle`)              | Strong grammar support, but the Svelte integration and type generation would be Mannie's to build       |

## Validation

V11.
