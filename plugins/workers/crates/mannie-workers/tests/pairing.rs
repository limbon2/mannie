//! Pairing (V6; plugins/workers/docs/pairing-and-link.md, pairing): a good
//! code pairs and both sides pin each other's keys; a wrong, expired or used
//! code is refused; the code's attempts run out; a key in the middle never
//! pairs; a paired worker refuses another master; a key that is not the
//! pinned master's reaches only pairing; removing a reachable worker unpairs
//! it; the owner's name for a worker replaces its configured name, and a
//! refused name leaves the code unused.

#![cfg(test)]

use std::fs;

use mannie_link::keys::{KeyPin, LinkKey};
use mannie_link_worker::{CODE_FILE, PIN_FILE};
use mannie_plugin_sdk::contracts::WorkerId;
use mannie_workers::commands::WorkersError;
use mannie_workers::view::WorkerStatus;
use mannie_workers_testing::master::{LinkedMaster, get_with_key};
use mannie_workers_testing::worker::{KeyInTheMiddle, LinkedWorker, Relay};

/// The pin that the worker keeps of its master, if any.
fn master_pin_of(worker: &LinkedWorker) -> Option<String> {
    fs::read_to_string(worker.data().join(PIN_FILE)).ok()
}

#[tokio::test]
async fn a_good_code_pairs_and_both_sides_pin_each_others_keys() {
    let worker = LinkedWorker::start().await;
    let master = LinkedMaster::start().await;
    let code = worker.new_code();

    let paired = master.pair(&worker.address, &code).await.unwrap();
    let connected = master.wait_for_status(WorkerStatus::Connected).await;

    assert_eq!(paired.name, "build-01");
    assert_eq!(connected.id, paired.id);
    assert_eq!(connected.key_pin, worker.key.pin().short());
    let stored = master.workers.paired_workers().await.unwrap();
    assert_eq!(stored[0].key_pin, worker.key.pin().as_str());
    let pinned = master_pin_of(&worker).expect("the worker pinned the master");
    let master_key = master.key_pin();
    assert_eq!(KeyPin::parse(&pinned), Some(master_key));
    assert!(
        !worker.data().join(CODE_FILE).exists(),
        "the code works once"
    );
}

#[tokio::test]
async fn a_wrong_code_is_refused_and_the_attempts_run_out() {
    let worker = LinkedWorker::start().await;
    let master = LinkedMaster::start().await;
    let code = worker.new_code();
    let wrong = "0000-0000-0000-0000-0000";

    let mut refusals = Vec::new();
    for _attempt in 0..5 {
        refusals.push(master.pair(&worker.address, wrong).await.unwrap_err());
    }
    let sixth = master.pair(&worker.address, wrong).await.unwrap_err();
    let good_after = master.pair(&worker.address, &code).await.unwrap_err();

    assert!(
        refusals
            .iter()
            .all(|refusal| *refusal == WorkersError::CodeRefused),
        "{refusals:?}"
    );
    assert_eq!(sixth, WorkersError::TooManyAttempts);
    assert_eq!(good_after, WorkersError::CodeRefused, "the code is gone");
    assert_eq!(master_pin_of(&worker), None);
}

/// Moves the creation time of the worker's code by `offset_millis`.
fn shift_code_creation(file: &std::path::Path, offset_millis: i64) {
    let mut written: serde_json::Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    let shifted = written["createdAt"].as_i64().unwrap() + offset_millis;
    written["createdAt"] = serde_json::json!(shifted);
    fs::write(file, serde_json::to_vec(&written).unwrap()).unwrap();
}

#[tokio::test]
async fn an_expired_code_is_refused() {
    let worker = LinkedWorker::start().await;
    let master = LinkedMaster::start().await;
    let code = worker.new_code();
    let file = worker.data().join(CODE_FILE);
    shift_code_creation(&file, -11 * 60 * 1_000);

    let refused = master.pair(&worker.address, &code).await;

    assert_eq!(refused.unwrap_err(), WorkersError::CodeRefused);
    assert!(!file.exists(), "an expired code is deleted");
}

#[tokio::test]
async fn a_code_made_after_now_is_refused_because_the_clock_went_back() {
    let worker = LinkedWorker::start().await;
    let master = LinkedMaster::start().await;
    let code = worker.new_code();
    let file = worker.data().join(CODE_FILE);
    shift_code_creation(&file, 60 * 60 * 1_000);

    let refused = master.pair(&worker.address, &code).await;

    assert_eq!(refused.unwrap_err(), WorkersError::CodeRefused);
    assert!(!file.exists(), "a code of unknown age is deleted");
}

#[tokio::test]
async fn a_paired_worker_refuses_another_master_and_a_code_works_once() {
    let worker = LinkedWorker::start().await;
    let first = LinkedMaster::start().await;
    let second = LinkedMaster::start().await;
    let code = worker.new_code();
    first.pair(&worker.address, &code).await.unwrap();

    let again = second.pair(&worker.address, &code).await;
    let fresh = worker.new_code();
    let with_a_new_code = second.pair(&worker.address, &fresh).await;

    assert_eq!(again.unwrap_err(), WorkersError::AlreadyPaired);
    assert_eq!(with_a_new_code.unwrap_err(), WorkersError::AlreadyPaired);
}

#[tokio::test]
async fn a_key_in_the_middle_never_pairs_either_side() {
    let worker = LinkedWorker::start().await;
    let master = LinkedMaster::start().await;
    let middle = KeyInTheMiddle::start(&worker.address).await;
    let code = worker.new_code();

    let refused = master.pair(&middle.address, &code).await;

    assert_eq!(refused.unwrap_err(), WorkersError::CodeRefused);
    assert_eq!(master_pin_of(&worker), None, "the worker pinned nothing");
    assert_eq!(master.workers.paired_workers().await.unwrap().len(), 0);
}

#[tokio::test]
async fn a_key_that_is_not_the_pinned_masters_reaches_only_pairing() {
    let worker = LinkedWorker::start().await;
    let master = LinkedMaster::start().await;
    let code = worker.new_code();
    master.pair(&worker.address, &code).await.unwrap();
    master.wait_for_status(WorkerStatus::Connected).await;

    let stranger = LinkKey::generate().unwrap();
    let refused = get_with_key(&stranger, &worker.address, worker.key.pin()).await;

    assert_eq!(refused, tonic::Code::PermissionDenied);
}

#[tokio::test]
async fn removing_a_reachable_worker_unpairs_it_for_another_master() {
    let worker = LinkedWorker::start().await;
    let first = LinkedMaster::start().await;
    let second = LinkedMaster::start().await;
    let paired = first
        .pair(&worker.address, &worker.new_code())
        .await
        .unwrap();
    first.wait_for_status(WorkerStatus::Connected).await;

    first.hub.remove(&paired.id).await.unwrap();
    let repaired = second.pair(&worker.address, &worker.new_code()).await;

    assert!(repaired.is_ok(), "{repaired:?}");
    assert_eq!(first.workers.paired_workers().await.unwrap().len(), 0);
    assert_eq!(
        first.hub.remove(&paired.id).await.unwrap_err(),
        WorkersError::WorkerNotFound
    );
}

#[tokio::test]
async fn the_owners_name_replaces_the_configured_name_and_survives_the_hello() {
    let worker = LinkedWorker::start().await;
    let master = LinkedMaster::start().await;

    let paired = master
        .pair_named(&worker.address, &worker.new_code(), " freebsd-01 ")
        .await
        .unwrap();
    let connected = master.wait_for_status(WorkerStatus::Connected).await;

    assert_eq!(paired.name, "freebsd-01");
    assert_eq!(
        connected.name, "freebsd-01",
        "the hello keeps the owner's name"
    );
    assert_eq!(connected.configured_name, "build-01");
    assert_eq!(connected.chosen_name.as_deref(), Some("freebsd-01"));
    let stored = master.workers.paired_workers().await.unwrap();
    assert_eq!(stored[0].chosen_name.as_deref(), Some("freebsd-01"));

    let renamed = master.hub.rename(&paired.id, "build box").await.unwrap();
    let cleared = master.hub.rename(&paired.id, " \n ").await.unwrap();

    assert_eq!(renamed.name, "build box");
    assert_eq!(cleared.name, "build-01");
    assert_eq!(cleared.chosen_name, None);
    assert_eq!(master.hub.worker_views().borrow()[0].name, "build-01");
    assert_eq!(
        master.workers.paired_workers().await.unwrap()[0].chosen_name,
        None
    );
    let unknown = WorkerId::parse("0199e0f2-0000-7000-b000-000000000999").unwrap();
    assert_eq!(
        master.hub.rename(&unknown, "anything").await.unwrap_err(),
        WorkersError::WorkerNotFound
    );
}

#[tokio::test]
async fn a_name_that_is_too_long_or_taken_is_refused_before_the_code_is_used() {
    let first = LinkedWorker::start().await;
    let second = LinkedWorker::start().await;
    let master = LinkedMaster::start().await;
    let first_view = master
        .pair(&first.address, &first.new_code())
        .await
        .unwrap();
    let code = second.new_code();

    let taken = master.pair_named(&second.address, &code, "BUILD-01").await;
    let too_long = master
        .pair_named(&second.address, &code, &"x".repeat(65))
        .await;
    let paired = master.pair_named(&second.address, &code, "build-02").await;

    assert_eq!(taken.unwrap_err(), WorkersError::NameTaken);
    assert_eq!(
        too_long.unwrap_err(),
        WorkersError::NameInvalid { max_characters: 64 }
    );
    assert_eq!(
        paired.unwrap().name,
        "build-02",
        "the refused names left the code unused"
    );
    assert_eq!(
        master
            .hub
            .rename(&first_view.id, "Build-02")
            .await
            .unwrap_err(),
        WorkersError::NameTaken
    );
    assert_eq!(
        master
            .hub
            .rename(&first_view.id, "Build-01")
            .await
            .unwrap()
            .name,
        "Build-01",
        "a worker may keep its own name in another case"
    );
}

#[tokio::test]
async fn pairing_without_a_credential_store_is_refused() {
    let worker = LinkedWorker::start().await;
    let master = LinkedMaster::start().await;
    master.keys.set_unavailable(true);

    let refused = master.pair(&worker.address, &worker.new_code()).await;

    assert_eq!(
        refused.unwrap_err(),
        WorkersError::CredentialStoreUnavailable
    );
}

#[tokio::test]
async fn a_worker_whose_key_changed_is_shown_and_never_used() {
    let worker = LinkedWorker::start().await;
    let relay = Relay::start(&worker.address).await;
    let master = LinkedMaster::start().await;
    master
        .pair(&relay.address, &worker.new_code())
        .await
        .unwrap();
    master.wait_for_status(WorkerStatus::Connected).await;
    // Another host answers at the worker's address, with a key of its own,
    // as after a reinstall or from an attacker on the network.
    let impostor = LinkedWorker::start().await;

    relay.set_target(&impostor.address);
    relay.cut();
    let view = master.wait_for_status(WorkerStatus::KeyChanged).await;

    assert_eq!(view.status, WorkerStatus::KeyChanged);
    assert_eq!(
        master_pin_of(&impostor),
        None,
        "the impostor pinned nothing"
    );
    assert_eq!(
        impostor.harness.model.reply_requests().len(),
        0,
        "the impostor got no work"
    );
}
