//! The system tools as a program sets them up: their registry, the policy
//! ceiling of the host, the lock of changes, the facts of the machine, the
//! folders that tasks may reach and the words of a command line.

pub use mannie_tools::{
    Ceiling, MutationLock, SystemFacts, ToolRegistry, canonical_folder, command_words,
};
