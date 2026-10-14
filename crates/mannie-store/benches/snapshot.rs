//! NFR-PERF-04 benchmark: opening a conversation of 10,000 parts.
//!
//! A conversation view opens with a snapshot of the latest turns, and earlier
//! turns load page by page (docs/features/chat/conversations.md). This
//! benchmark seeds 5,000 turns, each the owner's message and a reply, and
//! measures the snapshot, a page of earlier turns and a replay of recent
//! events. It fails when the snapshot takes longer than its share of the one
//! second that opening the conversation may take.
//!
//! Run it with `cargo bench -p mannie-store --features validation`.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_store::validation::seed_conversation;
use mannie_store::{EventsAfter, Store, StoreOptions};
use mannie_types::clock::SystemClock;
use mannie_types::ids::{ConversationId, ProfileId};

/// Turns of the seeded conversation: a message and a reply each.
const TURNS: u32 = 5_000;

/// What the view loads when it opens: the latest 50 turns, at most 400 parts.
const SNAPSHOT_TURNS: u32 = 50;
const SNAPSHOT_PARTS: u32 = 400;

/// How long the snapshot may take at the 95th percentile. Opening the view
/// may take one second in all (NFR-PERF-04); the store's part is a tenth.
const SNAPSHOT_LIMIT: Duration = Duration::from_millis(100);

/// How often each measurement repeats.
const REPEATS: usize = 20;

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "an index into a list of a few measurements"
    )]
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn milliseconds(duration: Duration) -> String {
    format!("{:.1} ms", duration.as_secs_f64() * 1000.0)
}

async fn measure<F, Fut>(mut work: F) -> Vec<Duration>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut durations = Vec::with_capacity(REPEATS);
    for _ in 0..REPEATS {
        let started = Instant::now();
        work().await;
        durations.push(started.elapsed());
    }
    durations.sort();
    durations
}

#[expect(
    clippy::print_stdout,
    reason = "the benchmark reports its results on the console"
)]
fn report(name: &str, durations: &[Duration]) {
    println!(
        "| {name} | {} / {} / {} |",
        milliseconds(percentile(durations, 0.50)),
        milliseconds(percentile(durations, 0.95)),
        milliseconds(durations.last().copied().unwrap_or_default()),
    );
}

#[expect(
    clippy::print_stdout,
    reason = "the benchmark reports its results on the console"
)]
async fn run() -> bool {
    let directory = tempfile::tempdir().expect("a temporary folder for the benchmark database");
    let options = StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock));
    let store =
        Store::open(&directory.path().join("benchmark.sqlite"), &options).expect("the store opens");
    let conversation =
        ConversationId::parse("01928c4f-0000-7000-8000-000000000001").expect("a valid ID");
    let profile = ProfileId::parse("01928c4f-0000-7000-8000-000000000002").expect("a valid ID");
    let mut counter = 0_u64;
    let seeding = Instant::now();
    seed_conversation(&store, &conversation, &profile, TURNS, || {
        counter += 1;
        format!("01928c4f-0001-7000-8000-{counter:012x}")
    })
    .await
    .expect("the conversation is seeded");
    let seeded_in = seeding.elapsed();

    let snapshots = measure(|| async {
        let snapshot = store
            .snapshot(&conversation, SNAPSHOT_TURNS, SNAPSHOT_PARTS)
            .await
            .expect("the snapshot reads")
            .expect("the conversation exists");
        assert_eq!(snapshot.turns.len(), SNAPSHOT_TURNS as usize);
    })
    .await;
    let pages = measure(|| async {
        let (turns, _) = store
            .earlier_turns(&conversation, TURNS - SNAPSHOT_TURNS, SNAPSHOT_TURNS)
            .await
            .expect("earlier turns read");
        assert_eq!(turns.len(), SNAPSHOT_TURNS as usize);
    })
    .await;
    let snapshot = store
        .snapshot(&conversation, 1, 2)
        .await
        .expect("the snapshot reads")
        .expect("the conversation exists");
    let replays = measure(|| async {
        let replay = store
            .events_after(&conversation, snapshot.through_seq - 100, 5_000)
            .await
            .expect("the events read");
        assert!(matches!(replay, EventsAfter::Events { .. }));
    })
    .await;

    println!("## Opening a conversation of {} parts", TURNS * 2);
    println!();
    println!("Seeded in {}.", milliseconds(seeded_in));
    println!();
    println!("| Measure | p50 / p95 / max |");
    println!("| --- | --- |");
    report(
        &format!("Snapshot of the latest {SNAPSHOT_TURNS} turns"),
        &snapshots,
    );
    report(&format!("Page of {SNAPSHOT_TURNS} earlier turns"), &pages);
    report("Replay of the latest 100 events", &replays);
    let fast_enough = percentile(&snapshots, 0.95) <= SNAPSHOT_LIMIT;
    println!();
    println!(
        "Snapshot within {} at the 95th percentile: {}",
        milliseconds(SNAPSHOT_LIMIT),
        if fast_enough { "yes" } else { "no" }
    );
    fast_enough
}

fn main() -> ExitCode {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    if runtime.block_on(run()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
