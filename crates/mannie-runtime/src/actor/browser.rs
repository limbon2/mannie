//! Site consent scoped to a run, including continuation operations for
//! browser requests paused before reaching an unapproved origin (ADR-0064).

use futures_util::future::BoxFuture;
use mannie_store::{Change, Durability, NewPart};
use mannie_tools::{BrowserContinuation, BrowserSiteConsent, Done, StopSignal, Verdict};
use mannie_types::clock::UtcMillis;
use mannie_types::ids::PartId;
use mannie_types::run::RunActivity;
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalRequest, BrowserAction, Decision, Interruption,
    ToolCallDocument, ToolCallStatus,
};
use mannie_types::transcript::PartKind;
use tokio::sync::mpsc;

use super::operations::aborted;
use super::{Ending, RunActor, StepCall};
use crate::approvals::Answer;
use crate::calls::Setup;

pub(super) struct BrowserRequests<'a> {
    incoming: mpsc::Receiver<BrowserContinuation>,
    setup: &'a Setup,
    plugin: String,
    tool: String,
    part: PartId,
    started_at: UtcMillis,
    dispatched: Vec<StepCall>,
    denied: bool,
}

impl<'a> BrowserRequests<'a> {
    pub(super) fn for_call(
        setup: &'a Setup,
        call: &StepCall,
        accesses: &[Access],
    ) -> (Option<mpsc::Sender<BrowserContinuation>>, Option<Self>) {
        let (Some(plugin), Some(started_at)) = (&call.document.plugin, call.document.started_at)
        else {
            return (None, None);
        };
        if !accesses.iter().any(|access| {
            matches!(
                access,
                Access::BrowserAction { .. } | Access::BrowserControl { .. }
            )
        }) {
            return (None, None);
        }
        let (sender, incoming) = mpsc::channel(1);
        (
            Some(sender),
            Some(Self {
                incoming,
                setup,
                plugin: plugin.clone(),
                tool: call.document.tool.clone(),
                part: call.part_id.clone(),
                started_at,
                dispatched: Vec::new(),
                denied: false,
            }),
        )
    }

    pub(super) async fn receive(browser: Option<&mut Self>) -> (&mut Self, BrowserContinuation) {
        if let Some(browser) = browser
            && let Some(request) = browser.incoming.recv().await
        {
            return (browser, request);
        }
        std::future::pending().await
    }

    /// A first visit needs its own durable operation before the service may
    /// continue the exact request. A visit already covered by a grant is
    /// part of the original journaled action.
    pub(super) async fn authorize(
        &mut self,
        actor: &mut RunActor,
        (access, mut reply): BrowserContinuation,
        execution: &mut BoxFuture<'static, Done>,
        abort: Option<&StopSignal>,
    ) -> Result<Option<Done>, Ending> {
        if access.action != BrowserAction::ContinueNavigation
            || !access.is_canonical()
            || reply.is_closed()
        {
            return Ok(None);
        }
        let accesses = [Access::from_browser(self.plugin.clone(), access)];
        let surroundings = actor.surroundings(self.setup).await?;
        let request = match actor.verdict(&accesses, &surroundings, None) {
            Verdict::Allowed(Decision::BrowserSite | Decision::Grant) => {
                let _closed = reply.send(true);
                return Ok(None);
            }
            Verdict::Ask(request) => request,
            _ => return Ok(None),
        };
        if reply.is_closed() {
            return Ok(None);
        }
        let mut call =
            actor.start_browser_continuation(&self.plugin, &self.tool, accesses.to_vec());
        let mut done = None;
        let canceled = async {
            tokio::select! {
                () = reply.closed() => {}
                () = aborted(abort) => {}
                ended = execution => done = Some(ended),
            }
        };
        let answer = actor
            .ask_until(&mut call, &accesses, &request, canceled)
            .await?;
        let running = actor.activity(RunActivity::RunningTool {
            part_id: self.part.clone(),
            started_at: self.started_at,
        });
        actor.pending.push(running);
        match answer {
            Some(Answer::Owner(
                choice @ (ApprovalChoice::AllowOnce | ApprovalChoice::AllowForConversation),
            )) => {
                let decision = if choice == ApprovalChoice::AllowForConversation {
                    actor.remember_browser_grants(&request).await?;
                    Decision::AllowedForConversation
                } else {
                    actor.remember_browser_sites(&accesses, true);
                    Decision::BrowserSite
                };
                if !reply.is_closed() {
                    let started_at = actor.now();
                    call.document.status = ToolCallStatus::Executing;
                    call.document.decision = Some(decision);
                    call.document.decided_at = Some(started_at);
                    call.document.started_at = Some(started_at);
                    actor.pending.push(call.update(Vec::new()));
                    actor.flush(Durability::Full).await?;
                    if reply.send(true).is_ok() {
                        self.dispatched.push(call);
                        return Ok(done);
                    }
                }
                call.document.status = ToolCallStatus::Canceled;
            }
            Some(Answer::Owner(ApprovalChoice::Deny)) => {
                actor.remember_browser_sites(&accesses, false);
                self.denied = true;
                call.document.status = ToolCallStatus::Denied;
                call.document.decision = Some(Decision::Denied);
                call.document.decided_at = Some(actor.now());
            }
            _ => call.document.status = ToolCallStatus::Canceled,
        }
        call.document.ended_at = Some(actor.now());
        actor.finish_call(&call);
        actor.flush(Durability::Full).await?;
        Ok(done)
    }

    pub(super) fn finish(&mut self, actor: &mut RunActor, succeeded: bool) {
        self.incoming.close();
        for mut call in self.dispatched.drain(..) {
            if succeeded {
                call.document.status = ToolCallStatus::Completed;
            } else {
                call.document.status = ToolCallStatus::Uncertain;
                call.document.interruption = Some(Interruption::AnswerLost {
                    plugin: self.plugin.clone(),
                });
            }
            call.document.ended_at = Some(actor.now());
            actor.finish_call(&call);
        }
    }

    pub(super) fn denied(&self) -> bool {
        self.denied
    }
}

impl RunActor {
    /// Saves an explicit site grant before releasing a browser action. A
    /// conversation grant is never copied into the run's separate cache.
    pub(super) async fn remember_browser_grants(
        &self,
        request: &ApprovalRequest,
    ) -> Result<(), Ending> {
        let store = &self.runtime.inner.store;
        let ids = &self.runtime.inner.ids;
        let grants = request
            .grants
            .iter()
            .cloned()
            .map(|scope| (ids.grant(), scope))
            .collect();
        store
            .add_grants(&self.conversation, grants)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "could not save browser site grants");
                Ending::Failed(mannie_types::run::RunFailure::Internal)
            })?;
        Ok(())
    }

    pub(super) fn remember_browser_sites(&mut self, accesses: &[Access], allowed: bool) {
        for access in accesses {
            if !self.browser_sites.iter().any(|site| site.covers(access))
                && let Some(site) = BrowserSiteConsent::of(access, allowed)
            {
                self.browser_sites.push(site);
            }
        }
    }

    fn start_browser_continuation(
        &mut self,
        plugin: &str,
        tool: &str,
        accesses: Vec<Access>,
    ) -> StepCall {
        let part_id = self.runtime.inner.ids.part();
        let mut document = ToolCallDocument::receiving(part_id.to_string(), tool.to_owned());
        document.plugin = Some(plugin.to_owned());
        document.accesses = accesses;
        self.pending.push(Change::StartPart {
            part: NewPart {
                id: part_id.clone(),
                turn_id: self.turn.clone(),
                run_id: self.run.clone(),
                attempt: self.attempt,
                kind: PartKind::ToolCall,
                failure: None,
                tool_call: Some(document.clone()),
            },
        });
        StepCall {
            part_id,
            document,
            arguments: None,
            text: String::new(),
            task: None,
        }
    }
}
