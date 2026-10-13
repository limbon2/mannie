//! V3 benchmark: one SQLite writer keeps up with 8 concurrent runs that each
//! stream 200 tokens per second, with batched deltas (ADR-0009, ADR-0012).
//!
//! Each simulated run turns its token stream into delta events and appends
//! them in batches with `Durability::Normal`, the way the runtime stores
//! streamed text. Every two seconds each run also commits one state change
//! with `Durability::Full`, the way journal entries are stored. The benchmark
//! reports the commit latency of both kinds and fails when the writer does not
//! keep up: a batch whose commit takes longer than the batch interval.
//!
//! Run it with `cargo bench -p mannie-store --features validation`.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mannie_store::validation::{ValidationEvent, ValidationEvents};
use mannie_store::{Durability, Store, StoreOptions};
use mannie_types::clock::SystemClock;
use tokio::time::MissedTickBehavior;

/// A load to simulate.
struct Scenario {
    name: &'static str,
    runs: u32,
    tokens_per_second: u32,
    batch_interval: Duration,
    duration: Duration,
    /// Whether the validation fails when the writer does not keep up.
    required: bool,
}

const SCENARIOS: [Scenario; 2] = [
    Scenario {
        name: "V3 target: 8 runs at 200 tokens/s, a batch every 250 ms",
        runs: 8,
        tokens_per_second: 200,
        batch_interval: Duration::from_millis(250),
        duration: Duration::from_secs(10),
        required: true,
    },
    Scenario {
        name: "headroom: 8 runs at 200 tokens/s, a batch every 33 ms",
        runs: 8,
        tokens_per_second: 200,
        batch_interval: Duration::from_millis(33),
        duration: Duration::from_secs(10),
        required: false,
    },
];

/// How often each run commits a state change with full durability.
const STATE_CHANGE_INTERVAL: Duration = Duration::from_secs(2);

/// What one run measured.
#[derive(Default)]
struct RunMeasurements {
    events: u32,
    batch_latencies: Vec<Duration>,
    state_change_latencies: Vec<Duration>,
}

struct Report {
    events: u32,
    stored_events: usize,
    batches: usize,
    batch_latencies: Vec<Duration>,
    state_change_latencies: Vec<Duration>,
}

impl Report {
    fn keeps_up(&self, scenario: &Scenario) -> bool {
        let worst = self.batch_latencies.last().copied().unwrap_or_default();
        let all_stored = self.stored_events == self.events as usize;
        all_stored && worst < scenario.batch_interval
    }
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "an index into a list of at most a few thousand measurements"
    )]
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

async fn simulate_run(events: ValidationEvents, run: u32, scenario: &Scenario) -> RunMeasurements {
    let conversation = format!("01928c4f-0000-7000-8000-{run:012}");
    let mut measurements = RunMeasurements::default();
    let mut ticks = tokio::time::interval(scenario.batch_interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let started = Instant::now();
    let mut last_state_change = started;
    let mut sequence = 0_u32;
    loop {
        ticks.tick().await;
        let elapsed = started.elapsed().min(scenario.duration);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "at most a few thousand tokens per run"
        )]
        let due = (elapsed.as_secs_f64() * f64::from(scenario.tokens_per_second)) as u32;
        let batch: Vec<ValidationEvent> = (sequence..due)
            .map(|token| ValidationEvent {
                conversation: conversation.clone(),
                sequence: token,
                payload: format!(" tok{}", token % 1000),
            })
            .collect();
        if !batch.is_empty() {
            let submitted = Instant::now();
            events
                .append(Durability::Normal, batch)
                .await
                .expect("a batch of deltas commits");
            measurements.batch_latencies.push(submitted.elapsed());
            sequence = due;
        }
        if last_state_change.elapsed() >= STATE_CHANGE_INTERVAL {
            let state_change = ValidationEvent {
                conversation: format!("{conversation}-state"),
                sequence: u32::try_from(measurements.state_change_latencies.len())
                    .expect("few state changes"),
                payload: "step completed".to_owned(),
            };
            let submitted = Instant::now();
            events
                .append(Durability::Full, vec![state_change])
                .await
                .expect("a state change commits");
            measurements
                .state_change_latencies
                .push(submitted.elapsed());
            last_state_change = Instant::now();
        }
        if elapsed >= scenario.duration {
            break;
        }
    }
    measurements.events = sequence;
    measurements
}

async fn run_scenario(scenario: &'static Scenario) -> Report {
    let directory = tempfile::tempdir().expect("a temporary folder for the benchmark database");
    let options = StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock));
    let store =
        Store::open(&directory.path().join("benchmark.sqlite"), &options).expect("the store opens");
    let events = ValidationEvents::create(&store)
        .await
        .expect("the benchmark table exists");

    let tasks: Vec<_> = (0..scenario.runs)
        .map(|run| tokio::spawn(simulate_run(events.clone(), run, scenario)))
        .collect();
    let mut report = Report {
        events: 0,
        stored_events: 0,
        batches: 0,
        batch_latencies: Vec::new(),
        state_change_latencies: Vec::new(),
    };
    for task in tasks {
        let measurements = task.await.expect("a simulated run finishes");
        report.events += measurements.events;
        report.batches += measurements.batch_latencies.len();
        report.batch_latencies.extend(measurements.batch_latencies);
        report
            .state_change_latencies
            .extend(measurements.state_change_latencies);
    }
    for run in 0..scenario.runs {
        let conversation = format!("01928c4f-0000-7000-8000-{run:012}");
        report.stored_events += events
            .sequences(&conversation)
            .await
            .expect("the events read back")
            .len();
    }
    report.batch_latencies.sort();
    report.state_change_latencies.sort();
    report
}

fn milliseconds(duration: Duration) -> String {
    format!("{:.1} ms", duration.as_secs_f64() * 1000.0)
}

#[expect(
    clippy::print_stdout,
    reason = "the benchmark reports its results on the console"
)]
fn print_report(scenario: &Scenario, report: &Report) {
    let batches = &report.batch_latencies;
    let state_changes = &report.state_change_latencies;
    println!("## {}", scenario.name);
    println!();
    println!("| Measure | Value |");
    println!("| --- | --- |");
    println!(
        "| Delta events written / read back | {} / {} |",
        report.events, report.stored_events
    );
    println!("| Batches (Normal) | {} |", report.batches);
    println!(
        "| Batch commit latency p50 / p95 / p99 / max | {} / {} / {} / {} |",
        milliseconds(percentile(batches, 0.50)),
        milliseconds(percentile(batches, 0.95)),
        milliseconds(percentile(batches, 0.99)),
        milliseconds(batches.last().copied().unwrap_or_default()),
    );
    println!(
        "| State change commit latency (Full) p50 / p95 / max | {} / {} / {} |",
        milliseconds(percentile(state_changes, 0.50)),
        milliseconds(percentile(state_changes, 0.95)),
        milliseconds(state_changes.last().copied().unwrap_or_default()),
    );
    let verdict = if report.keeps_up(scenario) {
        "yes"
    } else {
        "no"
    };
    println!(
        "| Keeps up (every batch commits within {}) | {verdict} |",
        milliseconds(scenario.batch_interval)
    );
    println!();
}

fn main() -> ExitCode {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let mut failed = false;
    for scenario in &SCENARIOS {
        let report = runtime.block_on(run_scenario(scenario));
        print_report(scenario, &report);
        if scenario.required && !report.keeps_up(scenario) {
            failed = true;
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
