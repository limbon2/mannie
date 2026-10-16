//! Workers (docs/features/workers/README.md): what the master and a worker
//! share about a worker's host, its policy ceiling and the link between them.

mod assignment;
mod ceiling;
mod commands;
mod host;
mod view;

pub use assignment::{ProfileSnapshot, TaskAssignment};
pub use ceiling::CeilingMode;
pub use commands::{
    MAX_WORKER_NAME_CHARACTERS, PairWorker, RenameWorker, WorkerRef, WorkersError,
    check_worker_name,
};
pub use host::{CeilingView, HostFacts, WorkerDetails};
pub use view::{WorkerListUpdate, WorkerStatus, WorkerView};
