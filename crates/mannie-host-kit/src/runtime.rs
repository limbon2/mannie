//! The agent runtime as a program runs it: its dependencies and
//! configuration, the agents and the instruction sections, the machine's
//! context, the tools' environment, the vault of lent credentials and the
//! texts that the model reads.

pub use mannie_runtime::{
    AgentDefinition, AgentRegistry, CONVERSATION_AGENT, CredentialVault, GENERAL,
    InstructionContext, InstructionRegistry, InstructionSection, MachineContext, MachineSettings,
    MentionRegistry, Requester, Runtime, RuntimeConfig, RuntimeDeps, ToolEnvironment, ToolsDeps,
    VaultError, checkpoint_section, texts,
};
