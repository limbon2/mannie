//! Readers of git's machine formats (plugins/git/docs/tools.md, design):
//! the plugin asks git for output that is meant for programs, with NUL or
//! separator bytes between fields and no quoting, and reads it here. No
//! reader runs git, so each is tested on git's output as text.

pub(crate) mod blame;
pub(crate) mod conflicts;
pub(crate) mod diff;
pub(crate) mod grep;
pub(crate) mod log;
pub(crate) mod refs;
pub(crate) mod remotes;
pub(crate) mod status;
pub(crate) mod tree;
pub(crate) mod worktrees;
