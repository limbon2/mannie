//! Tests of the updates module over a source of their own: what a check
//! finds, the downloads with the limit of their size, the schedule with its
//! switch, and the restart with its record. No test reads the releases, and
//! none installs anything.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mannie_plugin_sdk::BoxFuture;
use mannie_store::{SettledUpdate, Store, StoreOptions};
use mannie_types::clock::{ManualClock, UtcMillis};
use mannie_types::ids::UpdateId;
use mannie_types::updates::{
    ReleasePage, RestartToUpdateAnswer, UpdateFailureCode, UpdateOutcome, UpdateResult,
    UpdateState, UpdateStatus, UpdatesError, UpdatesOffReason,
};
use tokio::sync::{Notify, watch};

use super::source::{InstallError, Offer, Package, Progress, Source};
use super::{
    Automatic, Availability, CHECK_INTERVAL, FIRST_CHECK_DELAY, MAX_PACKAGE_BYTES,
    PROGRESS_STEP_BYTES, RunningWork, Updates, confirmation, lock,
};

/// The version that runs in the tests.
const RUNNING: &str = "1.0.0";
/// A newer version.
const NEWER: &str = "1.1.0";
/// A version that is newer still.
const NEWEST: &str = "1.2.0";
/// What the tests' clock shows when a test starts.
const STARTED_AT: i64 = 1_794_000_000_000;
/// How long a test waits for the end of a check before it fails.
const PATIENCE: Duration = Duration::from_secs(30);
/// Every reason for which a check or a download fails.
const FAILURES: [UpdateFailureCode; 5] = [
    UpdateFailureCode::Unreachable,
    UpdateFailureCode::NoUpdateInformation,
    UpdateFailureCode::NoPackageForThisInstallation,
    UpdateFailureCode::SignatureInvalid,
    UpdateFailureCode::DownloadFailed,
];

/// What the fake source answers to a check.
#[derive(Clone, Default)]
enum Answer {
    /// No version is newer than the running one.
    #[default]
    Nothing,
    /// A newer version.
    Release(Release),
    /// The check fails.
    Fails(UpdateFailureCode),
}

/// A newer version and how its download goes.
#[derive(Clone)]
struct Release {
    version: String,
    /// The lengths of the parts that arrive, in order. The fake reports only
    /// lengths, so a test of the limit holds no installer of that size.
    parts: Vec<usize>,
    /// The size that the server names.
    total: Option<u64>,
    /// Why the download fails after its parts arrived, when it does.
    failure: Option<UpdateFailureCode>,
}

impl Release {
    /// A small installer that downloads and verifies.
    fn of(version: &str) -> Self {
        Self {
            version: version.to_owned(),
            parts: vec![1024],
            total: Some(1024),
            failure: None,
        }
    }
}

/// A source that answers what a test set, and counts what the module asked
/// of it.
#[derive(Default)]
struct Fake {
    answer: Mutex<Answer>,
    checks: AtomicUsize,
    downloads: AtomicUsize,
    installs: Mutex<Vec<String>>,
    holds_checks: AtomicBool,
    holds_downloads: AtomicBool,
    resumed: Notify,
}

impl Fake {
    fn answers(&self, answer: Answer) {
        *lock(&self.answer) = answer;
    }

    fn offers(&self, version: &str) {
        self.answers(Answer::Release(Release::of(version)));
    }

    fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }

    fn downloads(&self) -> usize {
        self.downloads.load(Ordering::SeqCst)
    }

    fn installs(&self) -> Vec<String> {
        lock(&self.installs).clone()
    }

    /// Makes a check wait for `resume` before it answers.
    fn hold_checks(&self) {
        self.holds_checks.store(true, Ordering::SeqCst);
    }

    /// Makes a download wait for `resume` after its parts arrived.
    fn hold_downloads(&self) {
        self.holds_downloads.store(true, Ordering::SeqCst);
    }

    /// Lets the check or the download that waits go on, and holds no other.
    fn resume(&self) {
        self.holds_checks.store(false, Ordering::SeqCst);
        self.holds_downloads.store(false, Ordering::SeqCst);
        self.resumed.notify_one();
    }
}

/// The fake as the module reads it.
struct FakeSource(Arc<Fake>);

impl Source for FakeSource {
    fn check(&self) -> BoxFuture<'static, Result<Option<Box<dyn Offer>>, UpdateFailureCode>> {
        let fake = Arc::clone(&self.0);
        Box::pin(async move {
            fake.checks.fetch_add(1, Ordering::SeqCst);
            if fake.holds_checks.load(Ordering::SeqCst) {
                fake.resumed.notified().await;
            }
            let answer = lock(&fake.answer).clone();
            match answer {
                Answer::Nothing => Ok(None),
                Answer::Fails(code) => Err(code),
                Answer::Release(release) => {
                    let offer: Box<dyn Offer> = Box::new(FakeOffer { fake, release });
                    Ok(Some(offer))
                }
            }
        })
    }
}

struct FakeOffer {
    fake: Arc<Fake>,
    release: Release,
}

impl Offer for FakeOffer {
    fn version(&self) -> &str {
        &self.release.version
    }

    fn download(
        self: Box<Self>,
        mut progress: Progress,
    ) -> BoxFuture<'static, Result<Box<dyn Package>, UpdateFailureCode>> {
        Box::pin(async move {
            let Self { fake, release } = *self;
            fake.downloads.fetch_add(1, Ordering::SeqCst);
            for part in release.parts {
                progress(part, release.total);
                // A download of the updater waits for each part, and the
                // module checks the limit of the size each time it does.
                tokio::task::yield_now().await;
            }
            if fake.holds_downloads.load(Ordering::SeqCst) {
                fake.resumed.notified().await;
            }
            if let Some(code) = release.failure {
                return Err(code);
            }
            let package: Box<dyn Package> = Box::new(FakePackage {
                fake,
                version: release.version,
            });
            Ok(package)
        })
    }
}

struct FakePackage {
    fake: Arc<Fake>,
    version: String,
}

impl Package for FakePackage {
    fn version(&self) -> &str {
        &self.version
    }

    fn install(self: Box<Self>) -> Result<(), InstallError> {
        lock(&self.fake.installs).push(self.version.clone());
        Ok(())
    }
}

/// The module over the fake source, on the runtime of the test.
struct Fixture {
    fake: Arc<Fake>,
    clock: Arc<ManualClock>,
    updates: Updates,
    status: watch::Receiver<UpdateStatus>,
}

impl Fixture {
    /// The module in a Mannie of the version `RUNNING`.
    fn new() -> Self {
        Self::of_version(RUNNING)
    }

    fn of_version(version: &str) -> Self {
        let fake = Arc::new(Fake::default());
        let clock = Arc::new(ManualClock::new(UtcMillis::from_millis(STARTED_AT)));
        let updates = Updates::new(
            version,
            Availability::On(Arc::new(FakeSource(Arc::clone(&fake)))),
            Arc::<ManualClock>::clone(&clock),
            tokio::runtime::Handle::current(),
        );
        let status = updates.subscribe();
        Self {
            fake,
            clock,
            updates,
            status,
        }
    }

    fn state(&self) -> UpdateState {
        self.status.borrow().state.clone()
    }

    fn outcome(&self) -> Option<UpdateOutcome> {
        self.status.borrow().last_outcome.clone()
    }

    /// Starts a check as the owner does, and waits for its end.
    async fn check(&mut self) -> UpdateState {
        self.updates.check().unwrap();
        self.settled().await
    }

    /// Waits until no check and no download runs. A check shows `Checking`
    /// from the call that starts it, so this never returns the state from
    /// before that check.
    async fn settled(&mut self) -> UpdateState {
        let ended = self.status.wait_for(|status| {
            !matches!(
                status.state,
                UpdateState::Checking | UpdateState::Downloading { .. }
            )
        });
        let status = tokio::time::timeout(PATIENCE, ended)
            .await
            .unwrap()
            .unwrap();
        status.state.clone()
    }
}

/// Lets every task run as far as it can. The clock of the test's runtime is
/// paused, so this sleep ends once the runtime has nothing else to do.
async fn idle() {
    tokio::time::sleep(Duration::from_millis(1)).await;
}

fn started_at() -> UtcMillis {
    UtcMillis::from_millis(STARTED_AT)
}

fn ready(version: &str) -> UpdateState {
    UpdateState::Ready {
        version: version.to_owned(),
    }
}

fn failed(code: UpdateFailureCode) -> UpdateState {
    UpdateState::Failed {
        code,
        checked_at: started_at(),
    }
}

#[tokio::test(start_paused = true)]
async fn a_build_that_updates_itself_starts_idle_and_without_a_check() {
    let test = Fixture::new();

    assert_eq!(
        *test.status.borrow(),
        UpdateStatus {
            current_version: RUNNING.to_owned(),
            state: UpdateState::Idle { checked_at: None },
            last_outcome: None,
            needs_system_password: cfg!(target_os = "linux"),
        }
    );
    idle().await;
    assert_eq!(test.fake.checks(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_build_that_does_not_update_itself_is_off_and_refuses_a_check() {
    let clock = Arc::new(ManualClock::new(started_at()));
    let updates = Updates::new(
        RUNNING,
        Availability::Off(UpdatesOffReason::DevelopmentBuild),
        clock,
        tokio::runtime::Handle::current(),
    );
    let status = updates.subscribe();
    let off = UpdateState::Off {
        reason: UpdatesOffReason::DevelopmentBuild,
    };

    assert_eq!(status.borrow().state, off);
    assert_eq!(updates.check(), Err(UpdatesError::NotAvailableInThisBuild));

    updates.start_schedule();
    updates.set_automatic(true);
    tokio::time::sleep(FIRST_CHECK_DELAY + CHECK_INTERVAL).await;
    assert_eq!(status.borrow().state, off);
}

#[tokio::test(start_paused = true)]
async fn a_check_that_finds_nothing_new_shows_when_it_ended() {
    let mut test = Fixture::new();

    assert_eq!(
        test.check().await,
        UpdateState::Idle {
            checked_at: Some(started_at())
        }
    );

    test.clock.advance(Duration::from_hours(1));
    assert_eq!(
        test.check().await,
        UpdateState::Idle {
            checked_at: Some(UtcMillis::from_millis(STARTED_AT + 60 * 60 * 1000))
        }
    );
    assert_eq!(test.fake.checks(), 2);
    assert_eq!(test.fake.downloads(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_check_shows_that_it_runs() {
    let mut test = Fixture::new();
    test.fake.hold_checks();

    test.updates.check().unwrap();
    assert_eq!(test.state(), UpdateState::Checking);
    idle().await;
    assert_eq!(test.state(), UpdateState::Checking);

    test.fake.resume();
    assert_eq!(
        test.settled().await,
        UpdateState::Idle {
            checked_at: Some(started_at())
        }
    );
}

#[tokio::test(start_paused = true)]
async fn a_newer_version_downloads_at_once_and_is_ready() {
    let mut test = Fixture::new();
    test.fake.offers(NEWER);

    assert_eq!(test.check().await, ready(NEWER));
    assert_eq!(test.fake.downloads(), 1);
    assert_eq!(test.updates.ready_version().as_deref(), Some(NEWER));
    assert_eq!(test.updates.require_ready(NEWER), Ok(()));
    assert_eq!(test.fake.installs(), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn a_download_shows_its_version_before_its_first_part() {
    let mut test = Fixture::new();
    test.fake.answers(Answer::Release(Release {
        parts: Vec::new(),
        total: None,
        ..Release::of(NEWER)
    }));
    test.fake.hold_downloads();

    test.updates.check().unwrap();
    idle().await;
    assert_eq!(
        test.state(),
        UpdateState::Downloading {
            version: NEWER.to_owned(),
            received_bytes: 0,
            total_bytes: None,
        }
    );

    test.fake.resume();
    assert_eq!(test.settled().await, ready(NEWER));
}

#[tokio::test(start_paused = true)]
async fn a_download_shows_what_arrived_in_steps() {
    let mut test = Fixture::new();
    let step = usize::try_from(PROGRESS_STEP_BYTES).unwrap();
    test.fake.answers(Answer::Release(Release {
        parts: vec![step, 1],
        total: Some(PROGRESS_STEP_BYTES + 1),
        ..Release::of(NEWER)
    }));
    test.fake.hold_downloads();

    test.updates.check().unwrap();
    idle().await;
    // The last byte is less than a step, so the state still shows the first
    // part.
    assert_eq!(
        test.state(),
        UpdateState::Downloading {
            version: NEWER.to_owned(),
            received_bytes: u32::try_from(PROGRESS_STEP_BYTES).unwrap(),
            total_bytes: Some(u32::try_from(PROGRESS_STEP_BYTES + 1).unwrap()),
        }
    );

    test.fake.resume();
    assert_eq!(test.settled().await, ready(NEWER));
}

#[tokio::test(start_paused = true)]
async fn the_ready_version_again_downloads_nothing() {
    let mut test = Fixture::new();
    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));

    assert_eq!(test.check().await, ready(NEWER));
    assert_eq!(test.fake.checks(), 2);
    assert_eq!(test.fake.downloads(), 1);
}

#[tokio::test(start_paused = true)]
async fn another_version_replaces_the_installer_that_is_ready() {
    let mut test = Fixture::new();
    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));

    test.fake.offers(NEWEST);
    assert_eq!(test.check().await, ready(NEWEST));
    assert_eq!(test.fake.downloads(), 2);
    assert_eq!(test.updates.ready_version().as_deref(), Some(NEWEST));
    assert_eq!(
        test.updates.require_ready(NEWER),
        Err(UpdatesError::VersionNotReady)
    );
}

#[tokio::test(start_paused = true)]
async fn a_check_that_finds_nothing_newer_drops_the_installer_that_is_ready() {
    let mut test = Fixture::new();
    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));

    test.fake.answers(Answer::Nothing);
    assert_eq!(
        test.check().await,
        UpdateState::Idle {
            checked_at: Some(started_at())
        }
    );
    assert_eq!(test.updates.ready_version(), None);
}

#[tokio::test(start_paused = true)]
async fn a_failed_check_shows_its_code_and_when_it_failed() {
    for code in FAILURES {
        let mut test = Fixture::new();
        test.fake.answers(Answer::Fails(code));

        assert_eq!(test.check().await, failed(code));
        assert_eq!(test.fake.downloads(), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_download_shows_its_code_and_leaves_nothing_ready() {
    for code in FAILURES {
        let mut test = Fixture::new();
        test.fake.answers(Answer::Release(Release {
            failure: Some(code),
            ..Release::of(NEWER)
        }));

        assert_eq!(test.check().await, failed(code));
        assert_eq!(test.updates.ready_version(), None);
    }
}

#[tokio::test(start_paused = true)]
async fn a_check_after_a_failed_one_can_succeed() {
    let mut test = Fixture::new();
    test.fake
        .answers(Answer::Fails(UpdateFailureCode::Unreachable));
    assert_eq!(test.check().await, failed(UpdateFailureCode::Unreachable));

    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));
}

#[tokio::test(start_paused = true)]
async fn a_failure_keeps_the_installer_that_is_ready() {
    let mut test = Fixture::new();
    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));

    test.fake
        .answers(Answer::Fails(UpdateFailureCode::Unreachable));
    assert_eq!(test.check().await, ready(NEWER));

    test.fake.answers(Answer::Release(Release {
        failure: Some(UpdateFailureCode::SignatureInvalid),
        ..Release::of(NEWEST)
    }));
    assert_eq!(test.check().await, ready(NEWER));
    assert_eq!(test.updates.ready_version().as_deref(), Some(NEWER));
}

#[tokio::test(start_paused = true)]
async fn a_second_check_does_not_start_while_one_runs() {
    let mut test = Fixture::new();
    test.fake.hold_checks();
    test.updates.check().unwrap();
    idle().await;

    test.updates.check().unwrap();
    test.updates.set_automatic(true);
    idle().await;
    assert_eq!(test.fake.checks(), 1);

    test.fake.resume();
    assert_eq!(
        test.settled().await,
        UpdateState::Idle {
            checked_at: Some(started_at())
        }
    );
    idle().await;
    assert_eq!(test.fake.checks(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_check_does_not_start_while_an_installer_downloads() {
    let mut test = Fixture::new();
    test.fake.offers(NEWER);
    test.fake.hold_downloads();
    test.updates.check().unwrap();
    idle().await;

    test.updates.check().unwrap();
    idle().await;
    assert_eq!(test.fake.checks(), 1);
    assert_eq!(test.fake.downloads(), 1);

    test.fake.resume();
    assert_eq!(test.settled().await, ready(NEWER));
    assert_eq!(test.fake.checks(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_installer_of_exactly_the_limit_downloads() {
    let mut test = Fixture::new();
    let limit = usize::try_from(MAX_PACKAGE_BYTES).unwrap();
    test.fake.answers(Answer::Release(Release {
        parts: vec![limit - 1, 1],
        total: Some(MAX_PACKAGE_BYTES),
        ..Release::of(NEWER)
    }));

    assert_eq!(test.check().await, ready(NEWER));
}

#[tokio::test(start_paused = true)]
async fn a_download_that_passes_the_limit_fails() {
    let mut test = Fixture::new();
    let limit = usize::try_from(MAX_PACKAGE_BYTES).unwrap();
    test.fake.answers(Answer::Release(Release {
        parts: vec![limit, 1, 1],
        total: None,
        ..Release::of(NEWER)
    }));

    assert_eq!(
        test.check().await,
        failed(UpdateFailureCode::DownloadFailed)
    );
    assert_eq!(test.updates.ready_version(), None);
}

#[tokio::test(start_paused = true)]
async fn a_download_that_announces_more_than_the_limit_fails() {
    let mut test = Fixture::new();
    test.fake.answers(Answer::Release(Release {
        parts: vec![1],
        total: Some(MAX_PACKAGE_BYTES + 1),
        ..Release::of(NEWER)
    }));

    assert_eq!(
        test.check().await,
        failed(UpdateFailureCode::DownloadFailed)
    );
    assert_eq!(test.updates.ready_version(), None);
}

#[tokio::test(start_paused = true)]
async fn the_schedule_checks_a_minute_after_the_start_and_every_day_after_it() {
    let test = Fixture::new();
    let moment = Duration::from_secs(1);
    test.updates.follow_setting(Automatic::On);
    test.updates.start_schedule();

    tokio::time::sleep(FIRST_CHECK_DELAY.saturating_sub(moment)).await;
    assert_eq!(test.fake.checks(), 0);
    tokio::time::sleep(moment * 2).await;
    assert_eq!(test.fake.checks(), 1);

    tokio::time::sleep(CHECK_INTERVAL.saturating_sub(moment * 2)).await;
    assert_eq!(test.fake.checks(), 1);
    tokio::time::sleep(moment * 2).await;
    assert_eq!(test.fake.checks(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_schedule_downloads_a_newer_version() {
    let mut test = Fixture::new();
    test.fake.offers(NEWER);
    test.updates.follow_setting(Automatic::On);
    test.updates.start_schedule();

    tokio::time::sleep(FIRST_CHECK_DELAY + Duration::from_secs(1)).await;

    assert_eq!(test.settled().await, ready(NEWER));
}

#[tokio::test(start_paused = true)]
async fn the_schedule_does_not_check_while_the_switch_is_off() {
    let mut test = Fixture::new();
    test.updates.follow_setting(Automatic::Off);
    test.updates.start_schedule();

    tokio::time::sleep(FIRST_CHECK_DELAY + CHECK_INTERVAL * 2).await;
    assert_eq!(test.fake.checks(), 0);

    // The owner's own check does not need the switch.
    assert_eq!(
        test.check().await,
        UpdateState::Idle {
            checked_at: Some(started_at())
        }
    );
    assert_eq!(test.fake.checks(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_schedule_does_not_check_while_the_switch_was_not_read() {
    let mut test = Fixture::new();
    test.updates.start_schedule();

    tokio::time::sleep(FIRST_CHECK_DELAY + CHECK_INTERVAL * 2).await;
    assert_eq!(test.fake.checks(), 0);

    test.updates.follow_setting(Automatic::Unread);
    tokio::time::sleep(CHECK_INTERVAL).await;
    assert_eq!(test.fake.checks(), 0);

    assert_eq!(
        test.check().await,
        UpdateState::Idle {
            checked_at: Some(started_at())
        }
    );
    assert_eq!(test.fake.checks(), 1);
}

#[tokio::test(start_paused = true)]
async fn reading_the_switch_at_the_start_starts_no_check() {
    let test = Fixture::new();

    test.updates.follow_setting(Automatic::On);
    idle().await;

    assert_eq!(test.fake.checks(), 0);
}

#[tokio::test(start_paused = true)]
async fn turning_the_switch_on_checks_at_once_and_the_schedule_goes_on() {
    let test = Fixture::new();
    test.updates.follow_setting(Automatic::Off);
    test.updates.start_schedule();
    tokio::time::sleep(FIRST_CHECK_DELAY + Duration::from_secs(1)).await;
    assert_eq!(test.fake.checks(), 0);

    test.updates.set_automatic(true);
    idle().await;
    assert_eq!(test.fake.checks(), 1);

    // The switch was on already, so nothing was turned on.
    test.updates.set_automatic(true);
    idle().await;
    assert_eq!(test.fake.checks(), 1);

    tokio::time::sleep(CHECK_INTERVAL).await;
    assert_eq!(test.fake.checks(), 2);

    test.updates.set_automatic(false);
    tokio::time::sleep(CHECK_INTERVAL).await;
    assert_eq!(test.fake.checks(), 2);
}

#[tokio::test(start_paused = true)]
async fn nothing_starts_after_the_stop() {
    let test = Fixture::new();
    test.updates.follow_setting(Automatic::Off);
    test.updates.start_schedule();

    test.updates.stop();

    test.updates.check().unwrap();
    test.updates.set_automatic(true);
    test.updates.start_schedule();
    tokio::time::sleep(FIRST_CHECK_DELAY + CHECK_INTERVAL).await;
    assert_eq!(test.fake.checks(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_stop_ends_a_download_that_runs() {
    let test = Fixture::new();
    test.fake.offers(NEWER);
    test.fake.hold_downloads();
    test.updates.check().unwrap();
    idle().await;
    assert_eq!(test.fake.downloads(), 1);

    test.updates.stop();
    test.fake.resume();
    idle().await;

    assert_eq!(test.updates.ready_version(), None);
}

#[tokio::test(start_paused = true)]
async fn the_release_pages_are_on_github() {
    let mut test = Fixture::new();

    assert_eq!(
        test.updates.release_page(ReleasePage::Running).unwrap(),
        "https://github.com/limbon2/mannie/releases/tag/v1.0.0"
    );
    assert_eq!(
        test.updates.release_page(ReleasePage::Latest).unwrap(),
        "https://github.com/limbon2/mannie/releases/latest"
    );
    assert_eq!(
        test.updates.release_page(ReleasePage::Ready),
        Err(UpdatesError::VersionNotReady)
    );

    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));
    assert_eq!(
        test.updates.release_page(ReleasePage::Ready).unwrap(),
        "https://github.com/limbon2/mannie/releases/tag/v1.1.0"
    );
}

#[test]
fn the_restart_asks_first_while_work_runs() {
    let nothing = RunningWork {
        active_runs: 0,
        background_commands: 0,
        terminals: 0,
    };
    let everything = RunningWork {
        active_runs: 1,
        background_commands: 2,
        terminals: 3,
    };
    let one_terminal = RunningWork {
        terminals: 1,
        ..nothing
    };

    assert_eq!(confirmation(nothing, false), None);
    assert_eq!(
        confirmation(everything, false),
        Some(RestartToUpdateAnswer::ConfirmFirst {
            active_runs: 1,
            background_commands: 2,
            terminals: 3,
        })
    );
    assert_eq!(
        confirmation(one_terminal, false),
        Some(RestartToUpdateAnswer::ConfirmFirst {
            active_runs: 0,
            background_commands: 0,
            terminals: 1,
        })
    );
    assert_eq!(confirmation(everything, true), None);
}

/// The journal of installs in a store of its own. The store closes before
/// its folder goes.
struct Journal {
    store: Store,
    _directory: tempfile::TempDir,
}

impl Journal {
    fn open(clock: &Arc<ManualClock>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let options = StoreOptions::new(
            directory.path().join("backups"),
            Arc::<ManualClock>::clone(clock),
        );
        let store = Store::open(&directory.path().join("mannie.sqlite"), &options).unwrap();
        Self {
            store,
            _directory: directory,
        }
    }

    /// What a start of this version reads about an install that waited.
    async fn settled_by(&self, version: &str) -> Option<SettledUpdate> {
        self.store.resolve_updates(version).await.unwrap()
    }
}

fn update_id() -> UpdateId {
    UpdateId::parse("0199e0f2-7a1b-7c3d-8e4f-0123456789ab").unwrap()
}

fn another_update_id() -> UpdateId {
    UpdateId::parse("0199e0f2-7a1b-7c3d-8e4f-0123456789ac").unwrap()
}

fn outcome(result: UpdateResult) -> UpdateOutcome {
    UpdateOutcome {
        from_version: RUNNING.to_owned(),
        to_version: NEWER.to_owned(),
        result,
    }
}

#[tokio::test]
async fn a_version_that_is_not_ready_is_not_installed() {
    let mut test = Fixture::new();
    let journal = Journal::open(&test.clock);

    assert_eq!(
        test.updates.require_ready(NEWER),
        Err(UpdatesError::VersionNotReady)
    );
    assert_eq!(
        test.updates
            .choose(update_id(), NEWER, &journal.store)
            .await,
        Err(UpdatesError::VersionNotReady)
    );

    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));
    assert_eq!(
        test.updates.require_ready(NEWEST),
        Err(UpdatesError::VersionNotReady)
    );
    assert_eq!(
        test.updates
            .choose(update_id(), NEWEST, &journal.store)
            .await,
        Err(UpdatesError::VersionNotReady)
    );

    assert!(test.updates.take_chosen().is_none());
    assert_eq!(journal.settled_by(NEWEST).await, None);
}

#[tokio::test]
async fn the_restart_records_the_install_and_hands_over_the_installer_that_the_owner_saw() {
    let mut test = Fixture::new();
    let journal = Journal::open(&test.clock);
    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));

    test.updates
        .choose(update_id(), NEWER, &journal.store)
        .await
        .unwrap();

    assert_eq!(test.state(), ready(NEWER));
    let chosen = test.updates.take_chosen().unwrap();
    assert_eq!(chosen.id, update_id());
    assert_eq!(chosen.package.version(), NEWER);
    chosen.package.install().unwrap();
    assert_eq!(test.fake.installs(), [NEWER]);
    assert_eq!(
        journal.settled_by(NEWER).await,
        Some(SettledUpdate {
            outcome: outcome(UpdateResult::Completed),
            failure: None,
        })
    );
}

#[tokio::test]
async fn nothing_else_starts_once_an_install_is_chosen() {
    let mut test = Fixture::new();
    let journal = Journal::open(&test.clock);
    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));
    test.updates
        .choose(update_id(), NEWER, &journal.store)
        .await
        .unwrap();

    // A check shows `Checking` from the call that starts it.
    test.updates.check().unwrap();
    assert_eq!(test.state(), ready(NEWER));
    test.updates.set_automatic(true);
    assert_eq!(test.state(), ready(NEWER));
    assert_eq!(test.fake.checks(), 1);

    assert_eq!(
        test.updates
            .choose(another_update_id(), NEWER, &journal.store)
            .await,
        Err(UpdatesError::VersionNotReady)
    );
    assert_eq!(test.updates.take_chosen().unwrap().id, update_id());
}

#[tokio::test]
async fn the_restart_ends_a_check_that_runs() {
    let mut test = Fixture::new();
    let journal = Journal::open(&test.clock);
    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));
    test.fake.hold_checks();
    test.updates.check().unwrap();
    assert_eq!(test.state(), UpdateState::Checking);

    test.updates
        .choose(update_id(), NEWER, &journal.store)
        .await
        .unwrap();

    assert_eq!(test.state(), ready(NEWER));
    assert_eq!(test.updates.take_chosen().unwrap().package.version(), NEWER);
}

#[tokio::test]
async fn an_install_without_its_record_does_not_happen() {
    let mut test = Fixture::new();
    let journal = Journal::open(&test.clock);
    test.fake.offers(NEWER);
    assert_eq!(test.check().await, ready(NEWER));
    // The journal refuses a second record with the ID of an earlier one.
    journal
        .store
        .begin_update(&update_id(), RUNNING, NEWER)
        .await
        .unwrap();

    assert_eq!(
        test.updates
            .choose(update_id(), NEWER, &journal.store)
            .await,
        Err(UpdatesError::StorageFailed)
    );

    assert!(test.updates.take_chosen().is_none());
    assert_eq!(test.state(), ready(NEWER));
    assert_eq!(test.fake.installs(), Vec::<String>::new());

    // The installer stays ready for a restart that the journal records.
    test.updates
        .choose(another_update_id(), NEWER, &journal.store)
        .await
        .unwrap();
    assert_eq!(test.updates.take_chosen().unwrap().id, another_update_id());
}

#[tokio::test]
async fn the_start_of_the_new_version_shows_that_the_install_completed() {
    let test = Fixture::of_version(NEWER);
    let journal = Journal::open(&test.clock);
    journal
        .store
        .begin_update(&update_id(), RUNNING, NEWER)
        .await
        .unwrap();

    test.updates.settle(&journal.store).await;
    assert_eq!(test.outcome(), Some(outcome(UpdateResult::Completed)));

    test.updates.dismiss_outcome();
    assert_eq!(test.outcome(), None);
}

#[tokio::test]
async fn the_start_of_the_old_version_shows_that_the_install_failed() {
    let test = Fixture::new();
    let journal = Journal::open(&test.clock);
    journal
        .store
        .begin_update(&update_id(), RUNNING, NEWER)
        .await
        .unwrap();
    journal
        .store
        .note_update_failure(&update_id(), "the installer did not start")
        .await
        .unwrap();

    test.updates.settle(&journal.store).await;
    assert_eq!(test.outcome(), Some(outcome(UpdateResult::Failed)));

    test.updates.dismiss_outcome();
    assert_eq!(test.outcome(), None);
}

#[tokio::test]
async fn a_start_without_an_install_shows_no_result() {
    let test = Fixture::new();
    let journal = Journal::open(&test.clock);

    test.updates.settle(&journal.store).await;

    assert_eq!(test.outcome(), None);
}
