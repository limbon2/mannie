//! The SDK's major version (ADR-0034, decision 3).

/// The major version of this SDK. A plugin's manifest names the major that
/// the plugin was built for, and the host refuses a plugin for another major
/// with the code `sdk_incompatible`. An additive change keeps the major
/// (decision 14).
pub const SDK_MAJOR: u16 = 1;
