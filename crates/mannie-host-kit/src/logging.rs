//! Logging to rotating files, with the redaction of secrets and the crash
//! files (docs/architecture/security.md, rules for handling secrets).

pub use mannie_logging::{
    ErrorChain, LogConfig, LoggingError, LoggingGuard, REDACTED, RedactionGuard, Redactor, init,
};
