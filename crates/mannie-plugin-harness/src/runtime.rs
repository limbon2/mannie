//! The runtime's parts that plugin tests drive: its test harness with the
//! fake model, the harness of a worker, the helpers that script the model
//! and open stores, the runtime's handles of executors and credentials, the
//! refusals of a worker's runtime, and the texts that the model reads.

pub use mannie_runtime::testing::{
    FakeModel, Harness, HarnessOptions, MemoryVault, SeenPart, Transcript, WorkerHarness,
    WorkerOptions, add_plugin, credentials, executor_host, fast_config, open_store_with,
    remove_plugin, reply, start_master, test_tools, tool_calls, turn_off_plugin,
};
pub use mannie_runtime::{
    AgentRegistry, AssignError, CredentialVault, LeaseError, Runtime, RuntimeConfig, texts,
};
