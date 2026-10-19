//! The plugin host over the runtime's real consumers and providers
//! (ADR-0034, decisions 5 and 6): a plugin's executor and its tool reach the
//! runs that start while the plugin is on and leave them when it stops, and
//! the service `executor-host` is bound to the executor kind that equals the
//! plugin's ID, so the plugin's instance reports its tasks as its work.

#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::{HostInfo, HostProfile, StartContext};
use mannie_plugin_sdk::contracts::WorkItem;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::plugin::{Instance, Plugin, PluginEntry, Started};
use mannie_plugin_sdk::points::executors::{Executors, TaskExecutor, TaskSubmission};
use mannie_plugin_sdk::services::executor_host::{ExecutorHost, ExecutorHostApi};
use mannie_plugins::{PluginHost, PluginHostDeps, PluginList};
use mannie_runtime::testing::{
    FAKE_EXECUTOR_KIND, FAKE_EXECUTOR_TOOL, FakeExecutor, Harness, HarnessOptions, reply,
    tool_calls,
};
use mannie_types::chat::StartConversation;
use mannie_types::clock::SystemClock;
use mannie_types::plugins::PluginState;

/// The plugin `fake`, whose executor is the runtime's fake, of the kind
/// `fake`.
const MANIFEST: &str = "# The runtime's fake executor as a plugin.
id = \"fake\"
sdk = 1
order = 10

[rust]
entry = \"mannie-fake\"

[services]
# The rows of the executor's tasks, which its instance counts as its work.
executor-host = 1
";

/// A plugin whose executor is the runtime's fake.
#[derive(Debug)]
struct ExecutorPlugin {
    executor: Arc<FakeExecutor>,
}

impl Plugin for ExecutorPlugin {
    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        let executor: Arc<dyn TaskExecutor> = Arc::<FakeExecutor>::clone(&self.executor);
        let host = context.service::<ExecutorHost>();
        Box::pin(async move {
            let instance = ExecutorInstance { host: host? };
            Ok(Started::new(instance)
                .with_contributions(Contributions::new().add::<Executors>(executor)))
        })
    }
}

/// The running plugin, which counts the executor's tasks that have not
/// ended as its work.
#[derive(Debug)]
struct ExecutorInstance {
    host: Arc<dyn ExecutorHostApi>,
}

impl Instance for ExecutorInstance {
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        let host = Arc::clone(&self.host);
        Box::pin(async move {
            let tasks = host.active_tasks().await.unwrap();
            let count = u32::try_from(tasks.len()).unwrap();
            vec![WorkItem::new("active_tasks", count)]
        })
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(std::future::ready(()))
    }
}

async fn setup() -> (Harness, Arc<ExecutorPlugin>, PluginHost) {
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    let plugin = Arc::new(ExecutorPlugin {
        executor: FakeExecutor::new(),
    });
    let hosted: Arc<dyn Plugin> = Arc::<ExecutorPlugin>::clone(&plugin);
    let mut providers = harness.runtime.service_providers();
    providers.extend(mannie_store::service_providers(&harness.store));
    let host = PluginHost::new(PluginHostDeps {
        store: harness.store.clone(),
        plugins: PluginList::new(vec![PluginEntry::new(MANIFEST, hosted)]).unwrap(),
        providers,
        consumers: harness.runtime.contribution_consumers(),
        clock: Arc::new(SystemClock),
        host: HostInfo::new("0.3.0", HostProfile::Release),
    })
    .unwrap();
    (harness, plugin, host)
}

/// Whether a run that starts now is offered the fake executor's tool.
async fn offers_the_tool(harness: &Harness, marker: &str) -> bool {
    harness.model.push_reply(reply("Hello."));
    let started = harness
        .runtime
        .start_conversation(StartConversation::new(format!("{marker}: hello")))
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    harness.model.requests_for(marker)[0]
        .tools
        .iter()
        .any(|tool| tool.name == FAKE_EXECUTOR_TOOL)
}

/// Waits until the executor got its first submission, and returns it.
async fn first_submission(executor: &FakeExecutor) -> TaskSubmission {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Some(submission) = executor.submissions().into_iter().next() {
                return submission;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the executor got a submission in time")
}

#[tokio::test]
async fn a_plugins_executor_reaches_the_runs_while_it_is_on_and_leaves_them_when_it_stops() {
    let (harness, _plugin, host) = setup().await;

    let before = offers_the_tool(&harness, "BEFORE").await;
    let on = host.set_enabled(FAKE_EXECUTOR_KIND, true).await.unwrap();
    let while_on = offers_the_tool(&harness, "ON").await;
    host.set_enabled(FAKE_EXECUTOR_KIND, false).await.unwrap();
    let after = offers_the_tool(&harness, "AFTER").await;

    assert!(!before);
    assert_eq!(on.state, PluginState::On);
    assert!(while_on);
    assert!(!after);
}

#[tokio::test]
async fn the_executor_host_is_bound_to_the_kind_that_equals_the_plugins_id() {
    let (harness, plugin, host) = setup().await;
    host.set_enabled(FAKE_EXECUTOR_KIND, true).await.unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        r#"{"activity":"Giving the build","background":false,"instruction":"Build the backend.","target":"box-01","title":"Build it"}"#,
    )]));
    harness.model.push_reply(reply("The box built it."));

    let idle = host.work(FAKE_EXECUTOR_KIND).await.unwrap();
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("OWNER: build it on the box"))
        .await
        .unwrap();
    let submission = first_submission(&plugin.executor).await;
    let busy = host.work(FAKE_EXECUTOR_KIND).await.unwrap();
    mannie_runtime::testing::executor_host(&harness.runtime, FAKE_EXECUTOR_KIND)
        .publish_task_state(FakeExecutor::reported(&submission, 2, "Built."))
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    let ended = host.work(FAKE_EXECUTOR_KIND).await.unwrap();

    // The instance reports a count of 0 while no task runs, which the host
    // leaves out.
    assert_eq!(idle.items, Vec::new());
    assert_eq!(busy.items, vec![WorkItem::new("active_tasks", 1)]);
    assert_eq!(ended.items, Vec::new());
}
