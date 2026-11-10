//! Updates of the desktop app (docs/features/updates.md, ADR-0063): Mannie
//! checks the releases of its repository for a newer version, downloads the
//! installer in the background, and installs it when the owner chooses
//! "Restart and update". One task checks at a time, and what it found shows
//! in a status that the window subscribes to. An install is recorded in the
//! store's journal before Mannie quits, and runs after the core has shut
//! down.

mod release;
mod source;
#[cfg(test)]
mod tests;
#[cfg(feature = "validation")]
pub(crate) mod validation;

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::Poll;
use std::time::Duration;

use mannie_logging::ErrorChain;
use mannie_plugin_sdk::context::HostProfile;
use mannie_store::Store;
use mannie_types::clock::Clock;
use mannie_types::ids::UpdateId;
use mannie_types::updates::{
    ReleasePage, RestartToUpdateAnswer, UpdateFailureCode, UpdateState, UpdateStatus, UpdatesError,
    UpdatesOffReason,
};
use tauri::AppHandle;
use tokio::sync::watch;
use tokio::task::AbortHandle;

pub(crate) use self::release::bundle_mark;
use self::source::{Offer, Package, Source};
use crate::state::Core;

/// The largest installer that Mannie keeps (ADR-0063, decision 6). A download
/// that passes it, or that the server announces as larger, ends.
const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;

/// How much of a download arrives between two changes of the status.
const PROGRESS_STEP_BYTES: u64 = 256 * 1024;

/// How long after the start the schedule checks for the first time.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);

/// How long the schedule waits between two checks.
const CHECK_INTERVAL: Duration = Duration::from_hours(24);

/// The releases of Mannie's repository in the browser.
const RELEASES_PAGE: &str = "https://github.com/limbon2/mannie/releases";

/// Whether this build updates itself, and from where.
pub(crate) enum Availability {
    /// Checks read this source.
    On(Arc<dyn Source>),
    /// This build does not update itself.
    Off(UpdatesOffReason),
}

/// What the setting `updates.check_automatically` says, as far as Mannie
/// could read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Automatic {
    /// The schedule checks.
    On,
    /// Only the owner starts a check.
    Off,
    /// The settings could not be read, so the schedule does not check: the
    /// switch may be off.
    Unread,
}

/// What an install would interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunningWork {
    /// Replies that a model is writing.
    pub(crate) active_runs: u32,
    /// Background commands that run.
    pub(crate) background_commands: u32,
    /// Terminals with a running program.
    pub(crate) terminals: u32,
}

impl RunningWork {
    /// What runs in the core now.
    pub(crate) fn in_core(core: &Core) -> Self {
        Self {
            active_runs: core.active_run_count(),
            background_commands: core.background_command_count(),
            terminals: core.running_terminal_count(),
        }
    }

    fn any(self) -> bool {
        self.active_runs > 0 || self.background_commands > 0 || self.terminals > 0
    }
}

/// The updates of this app: what a check found, the installer that is ready,
/// and the one that the owner chose to install.
pub(crate) struct Updates {
    inner: Arc<Inner>,
    #[cfg(feature = "validation")]
    validation: Option<Arc<validation::Controls>>,
}

struct Inner {
    current_version: String,
    /// `None` in a build that does not update itself.
    source: Option<Arc<dyn Source>>,
    clock: Arc<dyn Clock>,
    /// The check and the schedule run here. The core starts outside of a
    /// runtime's context, so the handle is kept.
    tasks: tokio::runtime::Handle,
    status: watch::Sender<UpdateStatus>,
    state: Mutex<State>,
}

struct State {
    /// The check or download that runs.
    check: Option<CheckTask>,
    /// Numbers the checks, so that a check that was stopped changes nothing.
    checks_started: u64,
    schedule: Option<AbortHandle>,
    /// The verified installer of the version that the status shows as ready.
    ready: Option<Box<dyn Package>>,
    /// Set while the journal's record of an install is written. No check
    /// starts and no second install is accepted meanwhile.
    recording: bool,
    /// The installer that runs after the core has shut down.
    chosen: Option<Chosen>,
    automatic: Automatic,
    stopped: bool,
}

struct CheckTask {
    number: u64,
    task: AbortHandle,
}

struct Chosen {
    id: UpdateId,
    package: Box<dyn Package>,
}

/// What a check found.
enum Found {
    /// No version is newer than the running one.
    Nothing,
    /// The version that is ready already.
    SameAsReady,
    /// Another version, downloaded and verified.
    New(Box<dyn Package>),
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl fmt::Debug for Updates {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Updates")
            .field("status", &*self.inner.status.borrow())
            .finish_non_exhaustive()
    }
}

impl Updates {
    /// Updates for the running version `current_version`. Checks and the
    /// schedule run on `tasks`.
    pub(crate) fn new(
        current_version: &str,
        availability: Availability,
        clock: Arc<dyn Clock>,
        tasks: tokio::runtime::Handle,
    ) -> Self {
        let (source, state) = match availability {
            Availability::On(source) => (Some(source), UpdateState::Idle { checked_at: None }),
            Availability::Off(reason) => (None, UpdateState::Off { reason }),
        };
        let (status, _receiver) = watch::channel(UpdateStatus {
            current_version: current_version.to_owned(),
            state,
            last_outcome: None,
            // On Linux the package manager runs as root, so the system asks
            // for the password. The Windows installer installs for the owner
            // only and asks for nothing.
            needs_system_password: cfg!(target_os = "linux"),
        });
        Self {
            inner: Arc::new(Inner {
                current_version: current_version.to_owned(),
                source,
                clock,
                tasks,
                status,
                state: Mutex::new(State {
                    check: None,
                    checks_started: 0,
                    schedule: None,
                    ready: None,
                    recording: false,
                    chosen: None,
                    automatic: Automatic::Unread,
                    stopped: false,
                }),
            }),
            #[cfg(feature = "validation")]
            validation: None,
        }
    }

    /// The updates of the running app (ADR-0063, decision 5): a validation
    /// build reads a fake source that the end-to-end tests set, a development
    /// build and a release build that no package installed do not update
    /// themselves, and an installed release build reads the releases of
    /// Mannie's repository. `close_log` runs right before a started install
    /// ends the process.
    pub(crate) fn for_app(
        app: &AppHandle,
        profile: HostProfile,
        clock: Arc<dyn Clock>,
        close_log: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let current_version = env!("CARGO_PKG_VERSION");
        let tasks = tauri::async_runtime::handle().inner().clone();
        #[cfg(feature = "validation")]
        if profile == HostProfile::Validation {
            let controls = Arc::new(validation::Controls::default());
            let source: Arc<dyn Source> =
                Arc::new(validation::FakeSource::new(Arc::clone(&controls)));
            let mut updates = Self::new(current_version, Availability::On(source), clock, tasks);
            updates.validation = Some(controls);
            return updates;
        }
        let availability = match profile {
            HostProfile::Development => Availability::Off(UpdatesOffReason::DevelopmentBuild),
            _ => match release::Installation::of_this_binary() {
                Some(installation) => {
                    let source = release::ReleaseSource::new(app.clone(), installation, close_log);
                    Availability::On(Arc::new(source))
                }
                None => Availability::Off(UpdatesOffReason::NotInstalledFromPackage),
            },
        };
        Self::new(current_version, availability, clock, tasks)
    }

    /// The status now and after each change.
    pub(crate) fn subscribe(&self) -> watch::Receiver<UpdateStatus> {
        self.inner.status.subscribe()
    }

    /// Starts a check, unless a check or a download runs or an install was
    /// chosen. What it finds shows in the status.
    pub(crate) fn check(&self) -> Result<(), UpdatesError> {
        let source = self
            .inner
            .source
            .as_ref()
            .ok_or(UpdatesError::NotAvailableInThisBuild)?;
        start_check(&self.inner, source);
        Ok(())
    }

    /// Starts the schedule: a check one minute after the start and every 24
    /// hours, each only while the setting is on and was read.
    pub(crate) fn start_schedule(&self) {
        if self.inner.source.is_none() {
            return;
        }
        let mut state = lock(&self.inner.state);
        if state.schedule.is_some() || state.stopped {
            return;
        }
        let task = self
            .inner
            .tasks
            .spawn(run_schedule(Arc::downgrade(&self.inner)));
        state.schedule = Some(task.abort_handle());
    }

    /// Takes what the settings said at the start, without a check.
    pub(crate) fn follow_setting(&self, automatic: Automatic) {
        lock(&self.inner.state).automatic = automatic;
    }

    /// Follows a change of the setting. Turning it on checks at once.
    pub(crate) fn set_automatic(&self, on: bool) {
        let turned_on = {
            let mut state = lock(&self.inner.state);
            let before = state.automatic;
            state.automatic = if on { Automatic::On } else { Automatic::Off };
            on && before != Automatic::On
        };
        if !turned_on {
            return;
        }
        if let Some(source) = &self.inner.source {
            start_check(&self.inner, source);
        }
    }

    /// The version whose installer is ready.
    pub(crate) fn ready_version(&self) -> Option<String> {
        self.inner.ready_version()
    }

    /// Refuses a version that cannot be installed now: it is not the one
    /// that is ready, or an install was chosen already.
    pub(crate) fn require_ready(&self, version: &str) -> Result<(), UpdatesError> {
        if lock(&self.inner.state).can_install(version) {
            Ok(())
        } else {
            Err(UpdatesError::VersionNotReady)
        }
    }

    /// Chooses the ready installer of `version` for the install after the
    /// shutdown. The journal's record is written first (ADR-0063, decision
    /// 9): when it cannot be written, the installer stays ready and nothing
    /// is chosen.
    pub(crate) async fn choose(
        &self,
        id: UpdateId,
        version: &str,
        store: &Store,
    ) -> Result<(), UpdatesError> {
        {
            let mut state = lock(&self.inner.state);
            if !state.can_install(version) {
                return Err(UpdatesError::VersionNotReady);
            }
            // A check that still runs could replace or drop the installer
            // that the owner chose.
            if let Some(check) = state.check.take() {
                check.task.abort();
            }
            state.recording = true;
            self.inner.publish(UpdateState::Ready {
                version: version.to_owned(),
            });
        }
        let recorded = store
            .begin_update(&id, &self.inner.current_version, version)
            .await;
        let mut state = lock(&self.inner.state);
        state.recording = false;
        match recorded {
            Ok(()) => {
                state.chosen = state.ready.take().map(|package| Chosen { id, package });
                tracing::info!(
                    to_version = version,
                    "the owner chose to install the update"
                );
                Ok(())
            }
            Err(error) => {
                tracing::error!(
                    error = %ErrorChain(&error),
                    error.code = error.code(),
                    "could not record the install of the update; nothing installs"
                );
                Err(UpdatesError::StorageFailed)
            }
        }
    }

    /// Settles the installs that the journal still shows as running, from
    /// the version that runs now, and keeps the newest result for the status.
    pub(crate) async fn settle(&self, store: &Store) {
        match store.resolve_updates(&self.inner.current_version).await {
            Ok(Some(settled)) => {
                let outcome = settled.outcome;
                tracing::info!(
                    from_version = %outcome.from_version,
                    to_version = %outcome.to_version,
                    result = ?outcome.result,
                    failure = settled.failure.as_deref(),
                    "the last install of an update is settled"
                );
                self.inner.status.send_modify(|status| {
                    status.last_outcome = Some(outcome);
                });
            }
            Ok(None) => {}
            Err(error) => {
                tracing::error!(
                    error = %ErrorChain(&error),
                    error.code = error.code(),
                    "could not settle the installs of updates in the journal"
                );
            }
        }
    }

    /// Forgets the result of the last install, after the owner closed its
    /// notice.
    pub(crate) fn dismiss_outcome(&self) {
        self.inner
            .status
            .send_if_modified(|status| status.last_outcome.take().is_some());
    }

    /// The address of a release's page.
    pub(crate) fn release_page(&self, page: ReleasePage) -> Result<String, UpdatesError> {
        match page {
            ReleasePage::Ready => {
                let version = self.ready_version().ok_or(UpdatesError::VersionNotReady)?;
                Ok(page_of_version(&version))
            }
            ReleasePage::Running => Ok(page_of_version(&self.inner.current_version)),
            ReleasePage::Latest => Ok(format!("{RELEASES_PAGE}/latest")),
        }
    }

    /// Stops the check and the schedule when the core shuts down. A chosen
    /// installer stays for the install.
    pub(crate) fn stop(&self) {
        let mut state = lock(&self.inner.state);
        state.stopped = true;
        if let Some(check) = state.check.take() {
            check.task.abort();
        }
        if let Some(schedule) = state.schedule.take() {
            schedule.abort();
        }
    }

    fn take_chosen(&self) -> Option<Chosen> {
        lock(&self.inner.state).chosen.take()
    }
}

#[cfg(feature = "validation")]
impl Updates {
    /// Sets what the fake source of a validation build offers.
    pub(crate) fn set_validation_offer(
        &self,
        offer: mannie_types::validation::ValidationUpdateOffer,
    ) {
        if let Some(controls) = &self.validation {
            controls.set_offer(offer);
        }
    }

    /// Sets the result of a last install, as a start after an install would.
    pub(crate) fn set_validation_outcome(
        &self,
        outcome: Option<mannie_types::updates::UpdateOutcome>,
    ) {
        self.inner.status.send_modify(|status| {
            status.last_outcome = outcome;
        });
    }

    /// The versions whose install a validation build was asked for.
    pub(crate) fn validation_installs(&self) -> Vec<String> {
        self.validation
            .as_ref()
            .map(|controls| controls.installs())
            .unwrap_or_default()
    }

    /// Records an install instead of running it. `false` in a build without
    /// the fake source, which installs.
    fn record_install(&self, version: &str) -> bool {
        let Some(controls) = &self.validation else {
            return false;
        };
        controls.record_install(version);
        true
    }
}

impl Inner {
    /// Shows a state, unless the status shows it already. Callers may hold
    /// the lock of the state: the channel's own lock is never held while
    /// that one is taken.
    fn publish(&self, state: UpdateState) {
        self.status.send_if_modified(|status| {
            if status.state == state {
                return false;
            }
            status.state = state;
            true
        });
    }

    /// Shows how far the download of check `number` is, unless that check
    /// was stopped.
    fn publish_progress(&self, number: u64, version: &str, received: u64, total: Option<u64>) {
        let state = lock(&self.state);
        if !state.is_current(number) {
            return;
        }
        // The contract counts in 32 bits. The limit of a download's size
        // stays far below them.
        self.publish(UpdateState::Downloading {
            version: version.to_owned(),
            received_bytes: u32::try_from(received).unwrap_or(u32::MAX),
            total_bytes: total.and_then(|total| u32::try_from(total).ok()),
        });
    }

    fn ready_version(&self) -> Option<String> {
        lock(&self.state)
            .ready
            .as_ref()
            .map(|package| package.version().to_owned())
    }

    fn automatic(&self) -> Automatic {
        lock(&self.state).automatic
    }

    /// Ends check `number` with what it found. A check decides what is
    /// ready: another version replaces the installer, the same version keeps
    /// it, no newer version drops it, and a failure changes nothing.
    fn finish_check(&self, number: u64, found: Result<Found, UpdateFailureCode>) {
        let mut state = lock(&self.state);
        if !state.is_current(number) {
            return;
        }
        state.check = None;
        let failure = match found {
            Ok(Found::New(package)) => {
                tracing::info!(version = package.version(), "an update is ready to install");
                state.ready = Some(package);
                None
            }
            Ok(Found::SameAsReady) => {
                tracing::info!("the update that is ready is still the newest version");
                None
            }
            Ok(Found::Nothing) => {
                tracing::info!("no version is newer than the running one");
                state.ready = None;
                None
            }
            Err(code) => {
                tracing::warn!(code = code.code(), "the check for updates failed");
                Some(code)
            }
        };
        let checked_at = self.clock.now();
        let next = match (&state.ready, failure) {
            (Some(package), _) => UpdateState::Ready {
                version: package.version().to_owned(),
            },
            (None, Some(code)) => UpdateState::Failed { code, checked_at },
            (None, None) => UpdateState::Idle {
                checked_at: Some(checked_at),
            },
        };
        self.publish(next);
    }
}

impl State {
    fn is_current(&self, number: u64) -> bool {
        self.check
            .as_ref()
            .is_some_and(|check| check.number == number)
    }

    fn can_install(&self, version: &str) -> bool {
        !self.recording
            && self.chosen.is_none()
            && !self.stopped
            && self
                .ready
                .as_ref()
                .is_some_and(|package| package.version() == version)
    }
}

/// Starts a check, unless one runs, an install is being recorded or was
/// chosen, or the core shuts down.
fn start_check(inner: &Arc<Inner>, source: &Arc<dyn Source>) {
    let mut state = lock(&inner.state);
    if state.check.is_some() || state.recording || state.chosen.is_some() || state.stopped {
        return;
    }
    state.checks_started += 1;
    let number = state.checks_started;
    inner.publish(UpdateState::Checking);
    // The task is stored under the lock that it takes to finish, so it finds
    // its own number even when it finishes at once.
    let task = inner
        .tasks
        .spawn(run_check(Arc::clone(inner), Arc::clone(source), number));
    state.check = Some(CheckTask {
        number,
        task: task.abort_handle(),
    });
}

async fn run_check(inner: Arc<Inner>, source: Arc<dyn Source>, number: u64) {
    let found = find(&inner, source.as_ref(), number).await;
    inner.finish_check(number, found);
}

/// Asks the source, and downloads the installer of a version that is not
/// ready yet.
async fn find(
    inner: &Arc<Inner>,
    source: &dyn Source,
    number: u64,
) -> Result<Found, UpdateFailureCode> {
    let Some(offer) = source.check().await? else {
        return Ok(Found::Nothing);
    };
    let version = offer.version().to_owned();
    if inner.ready_version().as_deref() == Some(version.as_str()) {
        return Ok(Found::SameAsReady);
    }
    inner.publish_progress(number, &version, 0, None);
    let package = download_within_limit(inner, offer, number, version).await?;
    Ok(Found::New(package))
}

/// Downloads an offer's installer, shows the progress, and ends a download
/// that passes the limit of its size. The source collects the parts and only
/// reports their lengths, so the limit is checked each time the download
/// yields.
async fn download_within_limit(
    inner: &Arc<Inner>,
    offer: Box<dyn Offer>,
    number: u64,
    version: String,
) -> Result<Box<dyn Package>, UpdateFailureCode> {
    let over_limit = Arc::new(AtomicBool::new(false));
    let progress = {
        let inner = Arc::clone(inner);
        let over_limit = Arc::clone(&over_limit);
        let mut received: u64 = 0;
        let mut published: u64 = 0;
        move |part: usize, total: Option<u64>| {
            received = received.saturating_add(u64::try_from(part).unwrap_or(u64::MAX));
            if received > MAX_PACKAGE_BYTES || total.is_some_and(|total| total > MAX_PACKAGE_BYTES)
            {
                over_limit.store(true, Ordering::Relaxed);
                return;
            }
            if received - published >= PROGRESS_STEP_BYTES {
                published = received;
                inner.publish_progress(number, &version, received, total);
            }
        }
    };
    let mut downloading = offer.download(Box::new(progress));
    std::future::poll_fn(|context| {
        let polled = downloading.as_mut().poll(context);
        if over_limit.load(Ordering::Relaxed) {
            tracing::warn!(
                limit_bytes = MAX_PACKAGE_BYTES,
                "the download of the update passed the limit of its size and ended"
            );
            return Poll::Ready(Err(UpdateFailureCode::DownloadFailed));
        }
        polled
    })
    .await
}

/// Checks one minute after the start and every 24 hours, while the setting
/// is on and was read. A failed check is not repeated before the next one,
/// and a check that the owner started does not move the schedule.
async fn run_schedule(updates: Weak<Inner>) {
    tokio::time::sleep(FIRST_CHECK_DELAY).await;
    loop {
        let Some(inner) = updates.upgrade() else {
            break;
        };
        if inner.automatic() == Automatic::On
            && let Some(source) = &inner.source
        {
            start_check(&inner, source);
        }
        drop(inner);
        tokio::time::sleep(CHECK_INTERVAL).await;
    }
}

fn page_of_version(version: &str) -> String {
    format!("{RELEASES_PAGE}/tag/v{version}")
}

/// The answer that asks the owner first, when an install would interrupt
/// work and the owner did not confirm it yet.
pub(crate) fn confirmation(running: RunningWork, confirmed: bool) -> Option<RestartToUpdateAnswer> {
    if confirmed || !running.any() {
        return None;
    }
    Some(RestartToUpdateAnswer::ConfirmFirst {
        active_runs: running.active_runs,
        background_commands: running.background_commands,
        terminals: running.terminals,
    })
}

/// Records the install of the ready `version` and quits through the path of
/// "Quit". The install itself runs after the core has shut down
/// ([`install_chosen`]). A validation build with the fake source records the
/// request and keeps running.
pub(crate) async fn restart(
    app: &AppHandle,
    core: &Core,
    version: &str,
) -> Result<(), UpdatesError> {
    #[cfg(feature = "validation")]
    if core.updates().record_install(version) {
        return Ok(());
    }
    let (Some(store), Some(runtime)) = (core.store(), core.runtime()) else {
        return Err(UpdatesError::StoreUnavailable);
    };
    core.updates()
        .choose(runtime.new_update_id(), version, store)
        .await?;
    crate::window::close::quit(app);
    Ok(())
}

/// Opens the page of a release in the browser. The address is built from the
/// versions that the core holds, and the link is checked like every other.
pub(crate) fn open_release_page(core: &Core, page: ReleasePage) -> Result<(), UpdatesError> {
    let url = core.updates().release_page(page)?;
    crate::links::open(core, &url).map_err(|error| {
        tracing::warn!(
            error.code = error.code(),
            "the page of the release did not open"
        );
        UpdatesError::BrowserUnavailable
    })
}

/// Installs the update that the owner chose, after the event loop has ended
/// and the core has shut down (ADR-0063, decisions 8 and 10). On Windows a
/// started install ends the process. On Linux, and when the install fails,
/// Mannie starts itself again: the new version after an install, the old one
/// after a failure, which is logged and noted in the journal's record.
pub(crate) fn install_chosen(core: &Core, env: &tauri::Env) {
    let Some(chosen) = core.updates().take_chosen() else {
        return;
    };
    let to_version = chosen.package.version().to_owned();
    tracing::info!(%to_version, "installing the update");
    match chosen.package.install() {
        Ok(()) => tracing::info!(%to_version, "the update is installed; Mannie starts again"),
        Err(error) => {
            let failure = ErrorChain(error.as_ref()).to_string();
            tracing::error!(
                error = %failure,
                %to_version,
                "the update did not install; Mannie starts again"
            );
            note_failure(core, &chosen.id, &failure);
        }
    }
    if let Err(error) = start_again(env) {
        tracing::error!(%error, "Mannie could not start itself again");
    }
}

/// Notes why an install failed in the journal's record. The store still
/// writes after the core's shutdown.
fn note_failure(core: &Core, id: &UpdateId, failure: &str) {
    let Some(store) = core.store() else {
        return;
    };
    if let Err(error) = tauri::async_runtime::block_on(store.note_update_failure(id, failure)) {
        tracing::error!(
            error = %ErrorChain(&error),
            error.code = error.code(),
            "could not note the failed install in the journal"
        );
    }
}

/// Starts Mannie with the arguments that it was started with. Tauri's own
/// restart would end the process itself, before the log is closed.
fn start_again(env: &tauri::Env) -> std::io::Result<()> {
    let binary = tauri::process::current_binary(env)?;
    std::process::Command::new(binary)
        .args(env.args_os.iter().skip(1))
        .spawn()?;
    Ok(())
}
