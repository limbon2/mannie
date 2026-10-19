//! A plugin's definition (ADR-0034, decision 4): what its code implements,
//! what a start returns, and the entry that names the plugin with its
//! manifest for the list of plugins.
//!
//! The manifest, not the code, says who the plugin is: its ID, its order and
//! the services that it uses. The plugin's entry crate calls
//! [`export_plugin!`](crate::export_plugin), which embeds the text of its
//! folder's `plugin.toml` when the crate compiles, so the host parses and
//! checks the manifest before it calls any of the plugin's code.
//!
//! A start returns a [`Started`]: what the plugin contributes while it is
//! on, the handlers of its commands and channels, and the [`Instance`] that
//! runs until the host stops it. The host keeps the instance, asks it for the
//! plugin's work in progress, and stops it after it took the plugin's
//! contributions out of the core's consumers and its calls in flight ended.

use std::fmt;
use std::sync::Arc;

use crate::BoxFuture;
use crate::context::{DefaultContext, StartContext};
use crate::contracts::WorkItem;
use crate::contributions::Contributions;
use crate::error::StartFailure;
use crate::ipc::{Handlers, Interface};
use crate::services::store::StoreModule;

/// What a plugin's code implements.
pub trait Plugin: Send + Sync + fmt::Debug + 'static {
    /// The commands, channels and constants that the plugin's UI uses
    /// (decisions 8 and 9). The build grants each command to the main
    /// window, and the bindings exporter writes the plugin's TypeScript from
    /// it. Each start hands the host a handler for every command and channel
    /// that it lists. A plugin has none by default.
    fn interface(&self) -> Interface {
        Interface::new()
    }

    /// The plugin's module of the store, for a plugin with tables of its
    /// own. The host registers it whether or not the plugin is on, so the
    /// schema never depends on the plugin's state (ADR-0030, decision 5).
    /// The manifest declares the service `store` for it.
    fn store_module(&self) -> Option<StoreModule> {
        None
    }

    /// Whether the plugin starts when the owner has not chosen yet, which the
    /// host then keeps as the owner's choice (ADR-0030, decision 7). The
    /// context reads the plugin's module, so a default can follow the data
    /// that an older version left, as Workers' follows the paired workers.
    /// A plugin is off by default.
    fn default_on(&self, _context: DefaultContext) -> BoxFuture<'static, bool> {
        Box::pin(std::future::ready(false))
    }

    /// Starts the plugin with the services that its manifest declares, and
    /// returns what it contributes while it is on. A start that fails leaves
    /// nothing of the plugin running, and its code tells the owner why.
    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>>;
}

/// A plugin that started: what it contributes while it is on, the handlers
/// of its commands and channels, and the instance that the host keeps until
/// it stops the plugin.
#[derive(Debug)]
#[non_exhaustive]
pub struct Started {
    /// The plugin's entries of the extension points, which the host hands to
    /// the core's consumers all or none (decision 6).
    pub contributions: Contributions,
    /// The handlers of the commands and channels of the plugin's interface,
    /// which the host's dispatch calls while the plugin is on (decision 8).
    pub handlers: Handlers,
    /// The running plugin.
    pub instance: Box<dyn Instance>,
}

impl Started {
    /// A plugin that runs as `instance`, contributes nothing yet and has no
    /// handlers.
    pub fn new(instance: impl Instance) -> Self {
        Self {
            contributions: Contributions::new(),
            handlers: Handlers::new(),
            instance: Box::new(instance),
        }
    }

    /// The same start, contributing `contributions`.
    #[must_use]
    pub fn with_contributions(mut self, contributions: Contributions) -> Self {
        self.contributions = contributions;
        self
    }

    /// The same start, with `handlers` for the plugin's commands and
    /// channels.
    #[must_use]
    pub fn with_handlers(mut self, handlers: Handlers) -> Self {
        self.handlers = handlers;
        self
    }
}

/// A running plugin, which the host keeps from its start until it stops it.
pub trait Instance: Send + Sync + fmt::Debug + 'static {
    /// The plugin's work in progress, which turning the plugin off lets go,
    /// as items that the plugin's UI words, such as Workers' tasks that have
    /// not ended (decision 4). A plugin that cannot read its work logs why
    /// and reports none, so the switch still turns it off.
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        Box::pin(std::future::ready(Vec::new()))
    }

    /// Ends the plugin's background work, after the host took its
    /// contributions out of the core's consumers. Work in progress ends as
    /// it ends when Mannie quits, and nothing is repeated or lost.
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()>;
}

/// A plugin with its manifest, as the list of plugins holds it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PluginEntry {
    /// The text of the plugin's `plugin.toml`, which the host parses and
    /// checks before it calls the plugin.
    pub manifest: &'static str,
    /// The plugin.
    pub plugin: Arc<dyn Plugin>,
}

impl PluginEntry {
    /// The entry of `plugin`, whose manifest is `manifest`.
    pub fn new(manifest: &'static str, plugin: Arc<dyn Plugin>) -> Self {
        Self { manifest, plugin }
    }
}

/// Exports a plugin from its entry crate, `plugins/<id>/crates/<entry>/`:
/// `PLUGIN_MANIFEST`, the text of the folder's `plugin.toml`, and
/// `mannie_plugin()`, which makes the plugin with `$constructor` and returns
/// its entry. The entry crate calls it once, in its `lib.rs`, such as
/// `mannie_plugin_sdk::export_plugin!(NotesPlugin::new);`.
#[macro_export]
macro_rules! export_plugin {
    ($constructor:expr) => {
        /// The text of this plugin's `plugin.toml`, which the crate embeds
        /// when it compiles.
        pub const PLUGIN_MANIFEST: &str =
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../plugin.toml"));

        /// The plugin with its manifest, for the list of plugins.
        pub fn mannie_plugin() -> $crate::plugin::PluginEntry {
            $crate::plugin::PluginEntry::new(PLUGIN_MANIFEST, ::std::sync::Arc::new($constructor()))
        }
    };
}

// The tests read contributions and make contexts as the host does.
#[cfg(test)]
#[cfg(feature = "host")]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use futures_util::FutureExt;

    use super::{Instance, Plugin, PluginEntry, Started};
    use crate::BoxFuture;
    use crate::context::{DefaultContext, HostInfo, HostProfile, StartContext};
    use crate::contracts::WorkItem;
    use crate::contributions::{Contributions, ExtensionPoint, IdRule};
    use crate::error::StartFailure;

    /// A `Global` point whose entries are names.
    enum Names {}

    impl ExtensionPoint for Names {
        const NAME: &'static str = "names";
        const MAJOR: u16 = 1;
        const IDS: IdRule = IdRule::Global;
        type Entry = &'static str;

        fn id(entry: &Self::Entry) -> &str {
            entry
        }
    }

    #[derive(Debug, Default)]
    struct Counting {
        stops: Arc<AtomicU32>,
    }

    impl Instance for Counting {
        fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::ready(()))
        }
    }

    #[derive(Debug)]
    struct Quiet;

    impl Plugin for Quiet {
        fn start(
            &self,
            _context: StartContext,
        ) -> BoxFuture<'static, Result<Started, StartFailure>> {
            Box::pin(std::future::ready(Ok(Started::new(Counting::default()))))
        }
    }

    #[test]
    fn a_plugin_has_no_module_and_no_commands_and_is_off_unless_it_says_otherwise() {
        let plugin = Quiet;

        let default = plugin
            .default_on(DefaultContext::new(
                HostInfo::new("0.3.0", HostProfile::Release),
                None,
            ))
            .now_or_never()
            .unwrap();

        assert_eq!(plugin.store_module(), None);
        assert_eq!(plugin.interface().command_names(), Vec::<&str>::new());
        assert_eq!(plugin.interface().channel_names(), Vec::<&str>::new());
        assert!(!default);
    }

    #[test]
    fn a_start_carries_its_contributions_and_an_instance_that_reports_no_work_and_stops() {
        let stops = Arc::new(AtomicU32::new(0));
        let started = Started::new(Counting {
            stops: Arc::clone(&stops),
        })
        .with_contributions(Contributions::new().add::<Names>("one"));

        let names: Vec<&&str> = started.contributions.entries::<Names>().collect();
        let work: Vec<WorkItem> = started.instance.work().now_or_never().unwrap();
        started.instance.stop().now_or_never().unwrap();

        assert_eq!(names, [&"one"]);
        assert_eq!(started.handlers.command_names(), Vec::<&str>::new());
        assert_eq!(work, Vec::new());
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_entry_keeps_the_manifest_and_the_plugin() {
        let entry = PluginEntry::new("id = \"quiet\"", Arc::new(Quiet));

        assert_eq!(entry.manifest, "id = \"quiet\"");
        assert_eq!(format!("{:?}", entry.plugin), "Quiet");
    }
}
