# Browser changelog

Changes to the Browser plugin. Releases collect its entries in the [Mannie changelog](../../CHANGELOG.md).

## Unreleased

### Fixed

- Add a `browser` system-prompt section, update `browser_observe`, `browser_interact`, `browser_navigate`, `BrowserFillValue::Credential` schema docs, `mannie_inspect` target hints and the Browser management guide so models know to check saved site credentials and pass their 36-character UUID references, and return specific guidance instead of `navigation_denied` when a model tries plain-text `fill` on a password field or passes a credential label instead of a UUID.
