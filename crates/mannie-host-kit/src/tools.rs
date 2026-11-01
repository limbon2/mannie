//! The system tools as a program sets them up: their registry, the policy
//! ceiling of the host with its decisions on the databases registered on the
//! host and the digest of a write to one (ADR-0057, decisions 5 and 7), the
//! lock of changes, the facts of the machine, the folders that tasks may
//! reach and the words of a command line.

pub use mannie_tools::{
    Ceiling, DatabaseRequest, HostDecision, HostWrite, MutationLock, SystemFacts, ToolRegistry,
    WrittenStatement, canonical_folder, command_words, decide_database_request, write_digest,
};
