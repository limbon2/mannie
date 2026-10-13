use std::fs;
use std::path::Path;

use super::{DATABASE_FILE, Worker, WorkerOptions};

fn log_text(data_directory: &Path) -> String {
    let mut text = String::new();
    for entry in fs::read_dir(data_directory.join("logs")).unwrap() {
        text.push_str(&fs::read_to_string(entry.unwrap().path()).unwrap());
    }
    text
}

// Logging is global to the process, so this is the only unit test that starts
// a worker. The integration tests start the binary.
#[test]
fn starts_and_stops_in_order() {
    let folder = tempfile::tempdir().unwrap();
    let data_directory = folder.path().join("worker");
    let options = WorkerOptions {
        data_directory: data_directory.clone(),
        console_logging: false,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let worker = Worker::start(&options).unwrap();
        worker.stop();
    });

    assert!(
        data_directory.join(DATABASE_FILE).is_file(),
        "the store created its database"
    );
    let log = log_text(&data_directory);
    let order = [
        "the worker is starting",
        "the worker started",
        "the worker is stopping",
        "the worker stopped",
    ]
    .map(|message| {
        log.find(message)
            .unwrap_or_else(|| panic!("the log has no line {message:?}:\n{log}"))
    });
    assert!(
        order.is_sorted(),
        "the lines are in the order of the steps:\n{log}"
    );
}
