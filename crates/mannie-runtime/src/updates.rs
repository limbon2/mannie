//! IDs for the journal of installs of updates (ADR-0063, decision 9). The
//! desktop app records an install before it quits to run it, and the record's
//! ID comes from the runtime's generator like the ID of every other entity.

use mannie_types::ids::UpdateId;

use crate::Runtime;

impl Runtime {
    /// Allocates the ID of an install of an update using the runtime's
    /// injected clock.
    pub fn new_update_id(&self) -> UpdateId {
        self.inner.ids.update()
    }
}
