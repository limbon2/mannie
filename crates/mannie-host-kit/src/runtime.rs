//! The agent runtime as a program runs it: its dependencies and
//! configuration, the agents and the instruction sections, the machine's
//! context, the tools' environment, the vault of lent credentials, the texts
//! that the model reads, and the databases that the owner registered on the
//! host for the master's requests (ADR-0057).

pub use mannie_runtime::{
    AgentDefinition, AgentRegistry, CONVERSATION_AGENT, CredentialVault, GENERAL,
    HostDatabaseError, HostDatabaseSettings, HostDatabases, HostFile, HostWritten,
    InstructionContext, InstructionRegistry, InstructionSection, MachineContext, MachineSettings,
    MentionRegistry, Requester, Runtime, RuntimeConfig, RuntimeDeps, ToolEnvironment, ToolsDeps,
    VaultError, checkpoint_section, texts,
};
