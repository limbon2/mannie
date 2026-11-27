# Browser changelog

Changes to the Browser plugin. Releases collect its entries in the [Mannie changelog](../../CHANGELOG.md).

## Unreleased

### Fixed

- Respond without a client certificate to WebView2 client certificate requests instead of aborting the request, allowing sites with optional mutual TLS to load while keeping the Windows certificate store unused and the native picker suppressed.
