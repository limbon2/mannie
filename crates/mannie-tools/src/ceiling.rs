//! A worker's policy ceiling (docs/features/workers/host-and-ceiling.md, the
//! ceiling; ADR-0028): the most that its tools may do, as the owner set it in
//! the worker's configuration on its host, and the mode that a task narrowed
//! it to.

use std::path::PathBuf;

use mannie_types::tools::CommandRule;
use mannie_types::workers::CeilingMode;

/// The ceiling that the policy applies to every call on a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ceiling {
    /// The mode: read-only, ask or autonomous.
    pub mode: CeilingMode,
    /// The folders in which reads run without asking, and changes too in
    /// autonomous mode. Canonical, as the accesses are.
    pub allowed_folders: Vec<PathBuf>,
    /// The host's command rules, which let a command run without asking in
    /// autonomous mode.
    pub rules: Vec<CommandRule>,
}

impl Ceiling {
    /// The ceiling for a task that asks for `narrower`: the same folders and
    /// rules, and a mode that is never wider than this one's.
    #[must_use]
    pub fn narrowed(&self, narrower: Option<CeilingMode>) -> Self {
        Self {
            mode: self.mode.narrowed_by(narrower),
            ..self.clone()
        }
    }
}
