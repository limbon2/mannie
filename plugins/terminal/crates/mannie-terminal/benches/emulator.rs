//! The benchmark of the terminal plugin's headless emulator
//! (plugins/terminal/docs/adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md;
//! NFR-PERF-07): sessions of the kinds that S3 recorded, fed in reads of
//! 4 KiB as a pseudo console delivers them. It reports the throughput of
//! each, the text log that it left, and the size and time of the snapshots
//! that views get.
//!
//! It fails when the emulator parses plain or colored lines slower than
//! 4 MiB/s, about twice what the pseudo console of Windows delivered in a
//! flood (S1), since the emulator would then slow a program in the terminal.
//!
//! Run it with `task bench`.

use std::fmt::Write as _;
use std::hint::black_box;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mannie_terminal::validation::BenchEmulator;

/// The size of each read, as a pseudo console delivers output.
const READ_BYTES: usize = 4096;

/// The nonce of the benchmark's markers.
const NONCE: &str = "0123456789abcdef0123456789abcdef";

/// The text log's cap of one terminal.
const LOG_CAP: usize = 2 * 1024 * 1024;

/// The slowest throughput that keeps up with a pseudo console's flood.
const REQUIRED_MIB_PER_SECOND: f64 = 4.0;

/// A session to feed.
struct Session {
    name: &'static str,
    output: Vec<u8>,
    /// Whether the benchmark fails when the emulator is slower than
    /// [`REQUIRED_MIB_PER_SECOND`].
    required: bool,
}

/// What feeding a session measured.
struct Measured {
    duration: Duration,
    marks: usize,
    log_bytes: usize,
}

impl Measured {
    fn mib_per_second(&self, bytes: usize) -> f64 {
        mib(bytes) / self.duration.as_secs_f64().max(f64::EPSILON)
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the sizes are far below 2^52 bytes, and the report shows one decimal"
)]
fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// Lines of plain text, as `cat` of a log prints them.
fn plain_lines(count: usize) -> Vec<u8> {
    let mut output = String::new();
    for line in 0..count {
        let _written = write!(
            output,
            "{line:>7}  2026-10-30T12:00:00Z INFO service started a worker for the queue {}\r\n",
            line % 97
        );
    }
    output.into_bytes()
}

/// Lines with colors and styles, as a compiler's or `ls --color`'s output.
fn colored_lines(count: usize) -> Vec<u8> {
    let mut output = String::new();
    for line in 0..count {
        let _written = write!(
            output,
            "\x1b[1;33mwarning\x1b[0m: unused variable `value_{line}` in \x1b[4msrc/module_{}.rs\x1b[24m \x1b[38;5;{}m[{}]\x1b[0m \x1b[38;2;80;160;{}mtruecolor\x1b[0m\r\n",
            line % 31,
            line % 256,
            line % 7,
            line % 256
        );
    }
    output.into_bytes()
}

/// A progress bar that redraws one line, as a download prints it.
fn progress(updates: usize) -> Vec<u8> {
    let mut output = String::new();
    for update in 0..updates {
        let done = update % 101;
        let bar = "#".repeat(done / 5);
        let _written = write!(
            output,
            "\rDownloading [{bar:<20}] {done:>3}% {}.{} MiB/s\x1b[K",
            update % 40,
            update % 10
        );
        if done == 100 {
            output.push_str("\r\n");
        }
    }
    output.into_bytes()
}

/// Lines of wide characters and emoji.
fn wide_lines(count: usize) -> Vec<u8> {
    let mut output = String::new();
    for line in 0..count {
        let _written = write!(
            output,
            "{line:>6} 终端模拟器的测试行 ターミナル 🙂 터미널 ✓ — résumé naïve\r\n"
        );
    }
    output.into_bytes()
}

/// Commands at prompts that the setup marks, each with its output.
fn marked_commands(commands: usize, lines_each: usize) -> Vec<u8> {
    let mut output = String::new();
    for command in 0..commands {
        let _written = write!(
            output,
            "\x1b]633;P;Cwd=/home/owner/project;{NONCE}\x07\x1b]633;A;{NONCE}\x07owner@box:~/project$ \x1b]633;B;{NONCE}\x07ls -la build_{command}\r\n\x1b]633;E;ls -la build_{command};{NONCE}\x07\x1b]633;C;{NONCE}\x07"
        );
        for line in 0..lines_each {
            let _written = write!(
                output,
                "-rw-r--r-- 1 owner owner {:>8} Oct 30 12:00 artifact_{command}_{line}.o\r\n",
                (command * 31 + line) * 17
            );
        }
        let _written = write!(output, "\x1b]633;D;{};{NONCE}\x07", command % 3);
    }
    output.into_bytes()
}

/// Frames of a full-screen program on the alternate screen, as `top`
/// redraws them.
fn full_screen(frames: usize) -> Vec<u8> {
    let mut output = String::from("\x1b[?1049h");
    for frame in 0..frames {
        output.push_str("\x1b[H");
        for row in 1..=30 {
            let _written = write!(
                output,
                "\x1b[{row};1H\x1b[7m{:>5}\x1b[27m owner  20   0 {:>7}K {:>5}.{} \x1b[32mS\x1b[0m process_{}\x1b[K",
                1000 + row,
                (frame * row) % 99_999,
                row % 100,
                frame % 10,
                row
            );
        }
    }
    output.push_str("\x1b[?1049l");
    output.into_bytes()
}

fn sessions() -> Vec<Session> {
    vec![
        Session {
            name: "Plain lines, as cat of a log",
            output: plain_lines(200_000),
            required: true,
        },
        Session {
            name: "Colored lines, as a compiler's warnings",
            output: colored_lines(100_000),
            required: true,
        },
        Session {
            name: "A progress bar that redraws its line",
            output: progress(100_000),
            required: false,
        },
        Session {
            name: "Wide characters and emoji",
            output: wide_lines(100_000),
            required: false,
        },
        Session {
            name: "Marked commands, 2,000 with 10 lines each",
            output: marked_commands(2_000, 10),
            required: false,
        },
        Session {
            name: "A full-screen program on the alternate screen",
            output: full_screen(5_000),
            required: false,
        },
    ]
}

/// Feeds a session to a new emulator of 120 by 30 in reads of
/// [`READ_BYTES`].
fn feed(output: &[u8]) -> (BenchEmulator, Measured) {
    let mut emulator = BenchEmulator::new(120, 30, NONCE, LOG_CAP);
    let mut marks = 0;
    let started = Instant::now();
    for read in output.chunks(READ_BYTES) {
        marks += emulator.feed(black_box(read));
    }
    let duration = started.elapsed();
    let log_bytes = emulator.log_bytes();
    (
        emulator,
        Measured {
            duration,
            marks,
            log_bytes,
        },
    )
}

/// The time and size of a snapshot.
fn timed(snapshot: impl FnOnce() -> usize) -> (Duration, usize) {
    let started = Instant::now();
    let bytes = black_box(snapshot());
    (started.elapsed(), bytes)
}

fn milliseconds(duration: Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1000.0)
}

/// A size in KiB below 1 MiB, and in MiB from there.
fn size(bytes: usize) -> String {
    if bytes < 1024 * 1024 {
        format!("{:.1} KiB", mib(bytes) * 1024.0)
    } else {
        format!("{:.2} MiB", mib(bytes))
    }
}

#[expect(
    clippy::print_stdout,
    reason = "the benchmark reports its results on the console"
)]
fn main() -> ExitCode {
    let mut keeps_up = true;
    println!("## The headless emulator, fed in reads of 4 KiB");
    println!();
    println!("| Session | Size | Time | Throughput | Markers | Text log |");
    println!("| --- | --- | --- | --- | --- | --- |");
    let mut last_plain = None;
    for session in sessions() {
        let (emulator, measured) = feed(&session.output);
        let throughput = measured.mib_per_second(session.output.len());
        println!(
            "| {} | {} | {} | {throughput:.1} MiB/s | {} | {} |",
            session.name,
            size(session.output.len()),
            milliseconds(measured.duration),
            measured.marks,
            size(measured.log_bytes)
        );
        if session.required && throughput < REQUIRED_MIB_PER_SECOND {
            keeps_up = false;
        }
        if last_plain.is_none() {
            last_plain = Some(emulator);
        }
    }
    println!();
    if let Some(emulator) = last_plain {
        println!("## Snapshots after the plain lines, with a full text log");
        println!();
        println!("| Snapshot | Size | Time |");
        println!("| --- | --- | --- |");
        let snapshots = [
            (
                "Full, 2,000 lines of scrollback",
                timed(|| emulator.full_snapshot_bytes(2_000)),
            ),
            (
                "Full, 20,000 lines of scrollback",
                timed(|| emulator.full_snapshot_bytes(20_000)),
            ),
            (
                "The screen only",
                timed(|| emulator.screen_snapshot_bytes()),
            ),
        ];
        for (name, (duration, bytes)) in snapshots {
            println!("| {name} | {} | {} |", size(bytes), milliseconds(duration));
        }
        println!();
    }
    let verdict = if keeps_up { "yes" } else { "no" };
    println!(
        "Keeps up with a pseudo console (plain and colored lines at {REQUIRED_MIB_PER_SECOND} MiB/s or more): {verdict}"
    );
    if keeps_up {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
