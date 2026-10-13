//! Errors with their causes, for log fields.

use std::error::Error;
use std::fmt;

/// Formats an error followed by each of its sources, separated by `: `, so a
/// log line shows the whole cause and not only the outermost message.
///
/// ```
/// # use mannie_logging::ErrorChain;
/// # let error = std::io::Error::other("disk full");
/// tracing::error!(error = %ErrorChain(&error), "could not save the settings");
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ErrorChain<'a>(pub &'a (dyn Error + 'static));

impl fmt::Display for ErrorChain<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)?;
        let mut source = self.0.source();
        while let Some(cause) = source {
            write!(formatter, ": {cause}")?;
            source = cause.source();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ErrorChain;

    #[derive(Debug, thiserror::Error)]
    #[error("could not save the settings")]
    struct Outer(#[source] std::io::Error);

    #[test]
    fn shows_every_cause_in_order() {
        let error = Outer(std::io::Error::other("disk full"));

        assert_eq!(
            ErrorChain(&error).to_string(),
            "could not save the settings: disk full"
        );
    }
}
