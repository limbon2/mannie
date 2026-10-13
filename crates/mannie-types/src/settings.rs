//! Settings sections: the extension point through which a feature declares its
//! settings (docs/architecture/extension-points.md, settings sections).
//!
//! A section is a Rust type with defaults. Its fields hold only valid values,
//! because deserialization checks them, so a document that the store reads back
//! is valid by construction. The store persists each section as a versioned
//! JSON document under the section's key. Settings never contain secrets: a
//! credential is a reference to the credential store.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A typed settings section with defaults.
pub trait SettingsSection: Serialize + DeserializeOwned + Default + Send + Sync + 'static {
    /// The key the store keeps the section under: the owning feature's key,
    /// unique among sections and never reused for another section.
    const KEY: &'static str;

    /// The version of the section's document. A changed document shape gets
    /// the next version, and a reader refuses a document whose version is newer
    /// than the one it knows.
    const VERSION: u32;
}
