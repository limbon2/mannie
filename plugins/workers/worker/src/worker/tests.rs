use std::fs;
use std::path::Path;

use mannie_link_worker::KEY_FILE;

use super::{DATABASE_FILE, Worker, WorkerOptions};
use crate::config;
use crate::health::HEALTH_FILE;
use crate::paths::Folders;

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
fn starts_serves_and_stops_in_order() {
    let folder = tempfile::tempdir().unwrap();
    let data_directory = folder.path().join("worker");
    let folders = Folders {
        config: folder.path().join("etc"),
        data: data_directory.clone(),
        program: folder.path().join("program"),
    };
    let mut config = config::without_file("build-01", data_directory.clone(), &folders);
    config.listen = "127.0.0.1:0".parse().unwrap();
    let options = WorkerOptions {
        config,
        folders,
        console_logging: false,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let marker_while_running = runtime.block_on(async {
        let worker = Worker::start(&options).await.unwrap();
        let marker = crate::health::read(&data_directory);
        worker.run_until(async {}).await.unwrap();
        marker
    });

    assert!(
        data_directory.join(DATABASE_FILE).is_file(),
        "the store created its database"
    );
    assert!(
        data_directory.join(KEY_FILE).is_file(),
        "the worker made its key"
    );
    assert_eq!(
        marker_while_running.map(|health| health.version),
        Some(env!("CARGO_PKG_VERSION").to_owned()),
        "the running worker wrote its health marker"
    );
    assert!(
        !data_directory.join(HEALTH_FILE).exists(),
        "the stopped worker removed it"
    );
    let log = log_text(&data_directory);
    assert!(
        log.contains("the worker's link listens"),
        "the link served:\n{log}"
    );
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
