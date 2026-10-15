//! New IDs: UUIDv7 from the injected clock (docs/architecture/domain-model.md,
//! identifiers and time). A counter keeps IDs of the same millisecond in the
//! order they were made.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use mannie_types::clock::Clock;
use mannie_types::ids::{
    ArchiveId, CheckpointId, ConversationId, GrantId, InvalidId, PartId, ProfileId, ProjectId,
    RuleId, RunId, TurnId,
};
use uuid::timestamp::context::ContextV7;
use uuid::{Timestamp, Uuid};

/// Makes the IDs of new entities.
pub(crate) struct IdGenerator {
    clock: Arc<dyn Clock>,
    context: Mutex<ContextV7>,
}

impl fmt::Debug for IdGenerator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdGenerator")
            .finish_non_exhaustive()
    }
}

impl IdGenerator {
    /// A generator that takes the time from `clock`.
    pub(crate) fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            context: Mutex::new(ContextV7::new()),
        }
    }

    fn next(&self) -> String {
        let millis = self.clock.now().as_millis();
        // A clock before 1970 makes IDs at the epoch; their counter still
        // orders them.
        let millis = u64::try_from(millis).unwrap_or(0);
        let seconds = millis / 1000;
        let subsec_nanos = u32::try_from((millis % 1000) * 1_000_000).unwrap_or(0);
        let context = self.context.lock().unwrap_or_else(PoisonError::into_inner);
        let timestamp = Timestamp::from_unix(&*context, seconds, subsec_nanos);
        Uuid::new_v7(timestamp).to_string()
    }

    fn make<T>(&self, parse: fn(&str) -> Result<T, InvalidId>) -> T {
        parse(&self.next()).expect("the uuid crate writes UUIDs in their hyphenated form")
    }

    /// A new conversation ID.
    pub(crate) fn conversation(&self) -> ConversationId {
        self.make(ConversationId::parse)
    }

    /// A new turn ID.
    pub(crate) fn turn(&self) -> TurnId {
        self.make(TurnId::parse)
    }

    /// A new run ID.
    pub(crate) fn run(&self) -> RunId {
        self.make(RunId::parse)
    }

    /// A new part ID.
    pub(crate) fn part(&self) -> PartId {
        self.make(PartId::parse)
    }

    /// A new profile ID.
    pub(crate) fn profile(&self) -> ProfileId {
        self.make(ProfileId::parse)
    }

    /// A new checkpoint ID.
    pub(crate) fn checkpoint(&self) -> CheckpointId {
        self.make(CheckpointId::parse)
    }

    /// A new ID of an archived output.
    pub(crate) fn archive(&self) -> ArchiveId {
        self.make(ArchiveId::parse)
    }

    /// A new grant ID.
    pub(crate) fn grant(&self) -> GrantId {
        self.make(GrantId::parse)
    }

    /// A new rule ID.
    pub(crate) fn rule(&self) -> RuleId {
        self.make(RuleId::parse)
    }

    /// A new project ID.
    pub(crate) fn project(&self) -> ProjectId {
        self.make(ProjectId::parse)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_types::clock::{ManualClock, UtcMillis};

    use super::IdGenerator;

    #[test]
    fn ids_are_version_7_and_ordered_within_a_millisecond() {
        let clock = Arc::new(ManualClock::new(UtcMillis::from_millis(1_791_892_800_000)));
        let ids = IdGenerator::new(clock);

        let made: Vec<String> = (0..100).map(|_| ids.run().to_string()).collect();

        let mut sorted = made.clone();
        sorted.sort();
        assert_eq!(made, sorted);
        assert!(made.iter().all(|id| id.as_bytes()[14] == b'7'), "{made:?}");
        assert!(made[0].starts_with("01a13522-4200-7"), "{}", made[0]);
    }
}
