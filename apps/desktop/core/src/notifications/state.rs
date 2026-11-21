//! Pure state machine for desktop notifications, unread finished conversations
//! and tray badge state (ADR-0074). Kept independent of Tauri and SQLite so
//! every transition is covered by fast, deterministic unit tests.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use mannie_runtime::OwnerSignal;
use mannie_types::ids::{ConversationId, PartId};
use mannie_types::labels::clean_label;
use mannie_types::notifications::{
    MAX_NOTIFICATION_BODY_CHARACTERS, NotificationKind, NotificationLabels, NotificationSettings,
};

/// How many recent notification click targets are remembered in memory.
const MAX_REMEMBERED_TARGETS: usize = 256;

/// How many waiting `part_id` values are kept for duplicate suppression.
const MAX_NOTIFIED_PARTS: usize = 1_024;

/// A desktop notification ready to be delivered (or recorded in validation
/// builds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingToast {
    /// Identifier passed to `NotificationBuilder::id` so clicks resolve back
    /// to `conversation_id`.
    pub(crate) id: i32,
    /// What kind of event fired the notification.
    pub(crate) kind: NotificationKind,
    /// Fixed status title from the message catalog.
    pub(crate) title: String,
    /// Cleaned conversation title (never model output).
    pub(crate) body: String,
    /// Conversation that clicking this notification opens.
    pub(crate) conversation_id: ConversationId,
}

/// Side effects that a state transition asks the desktop driver to perform.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Effects {
    /// A desktop notification to show.
    pub(crate) dispatch: Option<PendingToast>,
    /// Mark this conversation unread in SQLite.
    pub(crate) persist_mark: Option<ConversationId>,
    /// Clear this conversation from SQLite unread storage.
    pub(crate) persist_clear: Option<ConversationId>,
    /// Updated full list of unread conversation IDs when the unread set changed.
    pub(crate) unread_changed: Option<Vec<ConversationId>>,
    /// New tray badge visibility when it changed.
    pub(crate) badge_changed: Option<bool>,
    /// New taskbar or dock badge count when it changed (`0` clears the badge).
    pub(crate) taskbar_badge_changed: Option<u32>,
}

/// Pure notification, taskbar count and tray-badge tracker.
#[derive(Debug)]
pub(crate) struct NotificationsModel {
    settings: NotificationSettings,
    labels: Option<NotificationLabels>,
    window_focused: bool,
    viewed_conversation: Option<ConversationId>,
    unread: BTreeSet<ConversationId>,
    waiting: HashSet<ConversationId>,
    notified_parts: HashSet<PartId>,
    notified_part_order: VecDeque<PartId>,
    next_notification_id: i32,
    targets: HashMap<i32, ConversationId>,
    target_order: VecDeque<i32>,
    badge_active: bool,
    taskbar_count: u32,
}

impl Default for NotificationsModel {
    fn default() -> Self {
        Self {
            settings: NotificationSettings::default(),
            labels: None,
            window_focused: false,
            viewed_conversation: None,
            unread: BTreeSet::new(),
            waiting: HashSet::new(),
            notified_parts: HashSet::new(),
            notified_part_order: VecDeque::new(),
            next_notification_id: 1,
            targets: HashMap::new(),
            target_order: VecDeque::new(),
            badge_active: false,
            taskbar_count: 0,
        }
    }
}

impl NotificationsModel {
    /// The current settings in memory.
    pub(crate) fn settings(&self) -> NotificationSettings {
        self.settings
    }

    /// The localized attention tooltip, once the window sent its labels.
    pub(crate) fn attention_tooltip(&self) -> Option<&str> {
        self.labels
            .as_ref()
            .map(|labels| labels.tray_attention_tooltip.as_str())
    }

    /// Whether the tray icon should currently show the badge.
    pub(crate) fn badge_active(&self) -> bool {
        self.badge_active
    }

    /// The count currently shown on the taskbar or dock badge (`0` when hidden).
    pub(crate) fn taskbar_badge_count(&self) -> u32 {
        self.taskbar_count
    }

    /// Whether the core considers the main window focused.
    #[cfg(feature = "validation")]
    pub(crate) fn window_focused(&self) -> bool {
        self.window_focused
    }

    /// The current unread finished conversations, sorted by ID.
    pub(crate) fn unread_ids(&self) -> Vec<ConversationId> {
        self.unread.iter().cloned().collect()
    }

    /// Looks up which conversation a clicked notification ID belongs to.
    pub(crate) fn target_for_notification(&self, id: i32) -> Option<ConversationId> {
        self.targets.get(&id).cloned()
    }

    /// Loads persisted unread finished conversations on startup.
    pub(crate) fn load_unread(&mut self, ids: Vec<ConversationId>) -> Effects {
        let mut effects = Effects::default();
        let before_unread = self.unread.clone();

        for id in ids {
            if self.is_viewing(&id) {
                effects.persist_clear = Some(id);
            } else {
                self.unread.insert(id);
            }
        }

        if self.unread != before_unread {
            effects.unread_changed = Some(self.unread_ids());
        }

        self.sync_badge(&mut effects);
        effects
    }

    /// Replaces the in-memory notification settings after they were saved or
    /// loaded.
    pub(crate) fn set_settings(&mut self, settings: NotificationSettings) -> Effects {
        self.settings = settings;

        let mut effects = Effects::default();
        self.sync_badge(&mut effects);
        effects
    }

    /// Stores localized notification titles and the badged tray tooltip.
    pub(crate) fn set_labels(&mut self, labels: NotificationLabels) -> bool {
        self.labels = Some(labels);
        self.badge_active
    }

    /// Updates whether the main window is visible and focused. If gaining
    /// focus reveals an unread finished conversation, clears it.
    pub(crate) fn set_window_focused(&mut self, focused: bool) -> Effects {
        self.window_focused = focused;

        let mut effects = Effects::default();

        if focused {
            self.clear_viewed_if_unread(&mut effects);
        }

        self.sync_badge(&mut effects);
        effects
    }

    /// Updates which conversation the main window currently displays. If the
    /// window is focused and the conversation is unread, clears it.
    pub(crate) fn set_viewed_conversation(
        &mut self,
        conversation_id: Option<ConversationId>,
    ) -> Effects {
        self.viewed_conversation = conversation_id;

        let mut effects = Effects::default();

        if self.window_focused {
            self.clear_viewed_if_unread(&mut effects);
        }

        self.sync_badge(&mut effects);
        effects
    }

    /// Applies one owner-conversation lifecycle signal from the runtime hub.
    pub(crate) fn on_signal(&mut self, signal: OwnerSignal) -> Effects {
        let mut effects = Effects::default();

        match signal {
            OwnerSignal::Active { conversation_id } | OwnerSignal::Deleted { conversation_id } => {
                self.waiting.remove(&conversation_id);
                self.remove_unread(&conversation_id, &mut effects);
            }
            OwnerSignal::EndedWithoutAlert { conversation_id } => {
                self.waiting.remove(&conversation_id);
            }
            OwnerSignal::AwaitingAnswer {
                conversation_id,
                title,
                part_id,
            } => {
                self.waiting.insert(conversation_id.clone());
                if self.remember_part(part_id) {
                    effects.dispatch =
                        self.maybe_toast(NotificationKind::Question, &conversation_id, &title);
                }
            }
            OwnerSignal::AwaitingApproval {
                conversation_id,
                title,
                part_id,
            } => {
                self.waiting.insert(conversation_id.clone());
                if self.remember_part(part_id) {
                    effects.dispatch =
                        self.maybe_toast(NotificationKind::Approval, &conversation_id, &title);
                }
            }
            OwnerSignal::Completed {
                conversation_id,
                title,
                ..
            } => {
                self.waiting.remove(&conversation_id);
                self.mark_finished_unread(&conversation_id, &mut effects);
                effects.dispatch =
                    self.maybe_toast(NotificationKind::Completed, &conversation_id, &title);
            }
            OwnerSignal::Failed {
                conversation_id,
                title,
                ..
            } => {
                self.waiting.remove(&conversation_id);
                self.mark_finished_unread(&conversation_id, &mut effects);
                effects.dispatch =
                    self.maybe_toast(NotificationKind::Failed, &conversation_id, &title);
            }
            OwnerSignal::Shutdown => {}
        }

        self.sync_badge(&mut effects);
        effects
    }

    fn is_viewing(&self, conversation_id: &ConversationId) -> bool {
        self.window_focused && self.viewed_conversation.as_ref() == Some(conversation_id)
    }

    fn clear_viewed_if_unread(&mut self, effects: &mut Effects) {
        if let Some(viewed) = self.viewed_conversation.clone() {
            self.remove_unread(&viewed, effects);
        }
    }

    fn remove_unread(&mut self, conversation_id: &ConversationId, effects: &mut Effects) {
        if self.unread.remove(conversation_id) {
            effects.persist_clear = Some(conversation_id.clone());
            effects.unread_changed = Some(self.unread_ids());
        }
    }

    fn mark_finished_unread(&mut self, conversation_id: &ConversationId, effects: &mut Effects) {
        if self.is_viewing(conversation_id) {
            return;
        }
        if self.unread.insert(conversation_id.clone()) {
            effects.persist_mark = Some(conversation_id.clone());
            effects.unread_changed = Some(self.unread_ids());
        }
    }

    fn remember_part(&mut self, part_id: PartId) -> bool {
        if !self.notified_parts.insert(part_id.clone()) {
            return false;
        }

        self.notified_part_order.push_back(part_id);
        while self.notified_part_order.len() > MAX_NOTIFIED_PARTS {
            if let Some(oldest) = self.notified_part_order.pop_front() {
                self.notified_parts.remove(&oldest);
            }
        }
        true
    }

    fn maybe_toast(
        &mut self,
        kind: NotificationKind,
        conversation_id: &ConversationId,
        raw_title: &str,
    ) -> Option<PendingToast> {
        if self.window_focused || !self.settings.allows(kind) {
            return None;
        }

        let title = self.labels.as_ref()?.title_for(kind).to_owned();
        let body = clean_label(raw_title, MAX_NOTIFICATION_BODY_CHARACTERS).unwrap_or_default();
        let id = self.allocate_target(conversation_id.clone());
        Some(PendingToast {
            id,
            kind,
            title,
            body,
            conversation_id: conversation_id.clone(),
        })
    }

    fn allocate_target(&mut self, conversation_id: ConversationId) -> i32 {
        let id = self.next_notification_id;
        self.next_notification_id = if id == i32::MAX { 1 } else { id + 1 };
        self.targets.insert(id, conversation_id);
        self.target_order.push_back(id);
        while self.target_order.len() > MAX_REMEMBERED_TARGETS {
            if let Some(oldest) = self.target_order.pop_front() {
                self.targets.remove(&oldest);
            }
        }
        id
    }

    fn attention_count(&self) -> u32 {
        let waiting_only = self
            .waiting
            .iter()
            .filter(|id| !self.unread.contains(*id))
            .count();
        let total = self.unread.len() + waiting_only;
        u32::try_from(total).unwrap_or(u32::MAX)
    }

    fn should_badge(&self) -> bool {
        self.settings.tray_badge && self.attention_count() > 0
    }

    fn desired_taskbar_count(&self) -> u32 {
        if self.settings.taskbar_badge {
            self.attention_count()
        } else {
            0
        }
    }

    fn sync_badge(&mut self, effects: &mut Effects) {
        let next_tray = self.should_badge();

        if next_tray != self.badge_active {
            self.badge_active = next_tray;
            effects.badge_changed = Some(next_tray);
        }

        let next_taskbar = self.desired_taskbar_count();

        if next_taskbar != self.taskbar_count {
            self.taskbar_count = next_taskbar;
            effects.taskbar_badge_changed = Some(next_taskbar);
        }
    }
}

#[cfg(test)]
mod tests {
    use mannie_runtime::OwnerSignal;
    use mannie_types::ids::{ConversationId, PartId, RunId};
    use mannie_types::notifications::{NotificationKind, NotificationLabels, NotificationSettings};

    use super::NotificationsModel;

    fn conv(last: u8) -> ConversationId {
        ConversationId::parse(&format!("0199e0f2-0000-7000-8000-0000000000{last:02x}")).unwrap()
    }

    fn run(last: u8) -> RunId {
        RunId::parse(&format!("0199e0f2-1000-7000-8000-0000000000{last:02x}")).unwrap()
    }

    fn part(last: u8) -> PartId {
        PartId::parse(&format!("0199e0f2-2000-7000-8000-0000000000{last:02x}")).unwrap()
    }

    fn labels() -> NotificationLabels {
        NotificationLabels {
            question_title: "Waiting for your answer".to_owned(),
            approval_title: "Approval needed".to_owned(),
            completed_title: "Reply finished".to_owned(),
            failed_title: "Reply failed".to_owned(),
            tray_attention_tooltip: "Mannie (attention needed)".to_owned(),
        }
    }

    #[test]
    fn unfocused_completion_notifies_marks_unread_and_clears_only_when_each_conversation_is_opened()
    {
        let mut model = NotificationsModel::default();
        model.set_labels(labels());
        model.set_viewed_conversation(Some(conv(1)));

        let first = model.on_signal(OwnerSignal::Completed {
            conversation_id: conv(1),
            title: "Build report\nwith control".to_owned(),
            run_id: run(1),
        });
        let toast = first.dispatch.expect("should dispatch completed toast");
        assert_eq!(toast.kind, NotificationKind::Completed);
        assert_eq!(toast.title, "Reply finished");
        assert_eq!(toast.body, "Build report with control");
        assert_eq!(first.persist_mark, Some(conv(1)));
        assert_eq!(first.badge_changed, Some(true));
        assert_eq!(model.target_for_notification(toast.id), Some(conv(1)));

        let second = model.on_signal(OwnerSignal::Failed {
            conversation_id: conv(2),
            title: "Deploy job".to_owned(),
            run_id: run(2),
        });
        assert_eq!(second.dispatch.unwrap().kind, NotificationKind::Failed);
        assert_eq!(second.persist_mark, Some(conv(2)));
        assert_eq!(second.badge_changed, None);
        assert_eq!(model.unread_ids(), vec![conv(1), conv(2)]);

        // Focusing Mannie clears conv(1) because it is open, but conv(2) keeps
        // the tray badge active until opened.
        let focused = model.set_window_focused(true);
        assert_eq!(focused.persist_clear, Some(conv(1)));
        assert_eq!(focused.unread_changed, Some(vec![conv(2)]));
        assert_eq!(focused.badge_changed, None);
        assert!(model.badge_active());

        // Opening conv(2) while focused clears the last unread conversation
        // and turns off the tray badge.
        let opened = model.set_viewed_conversation(Some(conv(2)));
        assert_eq!(opened.persist_clear, Some(conv(2)));
        assert_eq!(opened.unread_changed, Some(Vec::new()));
        assert_eq!(opened.badge_changed, Some(false));
        assert!(!model.badge_active());
    }

    #[test]
    fn focused_window_suppresses_toasts_and_marks_other_conversations_unread_only() {
        let mut model = NotificationsModel::default();
        model.set_labels(labels());
        model.set_window_focused(true);
        model.set_viewed_conversation(Some(conv(1)));

        // Completing the conversation already open in a focused window neither
        // notifies nor marks unread.
        let viewed = model.on_signal(OwnerSignal::Completed {
            conversation_id: conv(1),
            title: "Current chat".to_owned(),
            run_id: run(1),
        });
        assert!(viewed.dispatch.is_none());
        assert!(viewed.persist_mark.is_none());
        assert!(!model.badge_active());

        // Completing a different conversation while Mannie is focused sends no
        // OS toast, but marks it unread in the sidebar and badges the tray.
        let other = model.on_signal(OwnerSignal::Completed {
            conversation_id: conv(2),
            title: "Background chat".to_owned(),
            run_id: run(2),
        });
        assert!(other.dispatch.is_none());
        assert_eq!(other.persist_mark, Some(conv(2)));
        assert_eq!(other.badge_changed, Some(true));
    }

    #[test]
    fn waiting_parts_deduplicate_and_stops_never_notify_or_mark_unread() {
        let mut model = NotificationsModel::default();
        model.set_labels(labels());

        let first_q = model.on_signal(OwnerSignal::AwaitingAnswer {
            conversation_id: conv(1),
            title: "Question chat".to_owned(),
            part_id: part(1),
        });
        assert_eq!(first_q.dispatch.unwrap().kind, NotificationKind::Question);
        assert_eq!(first_q.badge_changed, Some(true));

        // A child approval shows over the question, then the same question
        // reappears; the second appearance of part(1) does not notify twice.
        let child_approval = model.on_signal(OwnerSignal::AwaitingApproval {
            conversation_id: conv(1),
            title: "Question chat".to_owned(),
            part_id: part(2),
        });
        assert_eq!(
            child_approval.dispatch.unwrap().kind,
            NotificationKind::Approval
        );

        let same_q = model.on_signal(OwnerSignal::AwaitingAnswer {
            conversation_id: conv(1),
            title: "Question chat".to_owned(),
            part_id: part(1),
        });
        assert!(same_q.dispatch.is_none());
        assert!(model.badge_active());

        // Stopping ends the wait without notifying or marking unread.
        let stopped = model.on_signal(OwnerSignal::EndedWithoutAlert {
            conversation_id: conv(1),
        });
        assert!(stopped.dispatch.is_none());
        assert!(stopped.persist_mark.is_none());
        assert_eq!(stopped.badge_changed, Some(false));
        assert!(!model.badge_active());
    }

    #[test]
    fn settings_switches_control_toasts_and_tray_badge_independently() {
        let mut model = NotificationsModel::default();
        model.set_labels(labels());
        model.set_settings(NotificationSettings {
            enabled: false,
            tray_badge: true,
            ..NotificationSettings::default()
        });

        let finished = model.on_signal(OwnerSignal::Completed {
            conversation_id: conv(1),
            title: "Silent job".to_owned(),
            run_id: run(1),
        });
        assert!(finished.dispatch.is_none());
        assert_eq!(finished.badge_changed, Some(true));
        assert_eq!(finished.taskbar_badge_changed, Some(1));

        // Turning off tray_badge leaves the taskbar count active.
        let tray_off = model.set_settings(NotificationSettings {
            enabled: false,
            taskbar_badge: true,
            tray_badge: false,
            ..NotificationSettings::default()
        });
        assert_eq!(tray_off.badge_changed, Some(false));
        assert_eq!(tray_off.taskbar_badge_changed, None);
        assert!(!model.badge_active());
        assert_eq!(model.taskbar_badge_count(), 1);
        // Unread state stays tracked in the sidebar even when the tray badge
        // switch is turned off.
        assert_eq!(model.unread_ids(), vec![conv(1)]);

        // Turning off taskbar_badge clears the taskbar count independently.
        let taskbar_off = model.set_settings(NotificationSettings {
            enabled: false,
            taskbar_badge: false,
            tray_badge: false,
            ..NotificationSettings::default()
        });
        assert_eq!(taskbar_off.taskbar_badge_changed, Some(0));
        assert_eq!(model.taskbar_badge_count(), 0);

        let both_back_on = model.set_settings(NotificationSettings {
            enabled: true,
            on_completion: false,
            taskbar_badge: true,
            tray_badge: true,
            ..NotificationSettings::default()
        });
        assert_eq!(both_back_on.badge_changed, Some(true));
        assert_eq!(both_back_on.taskbar_badge_changed, Some(1));
        assert!(model.badge_active());
        assert_eq!(model.taskbar_badge_count(), 1);
    }

    #[test]
    fn taskbar_count_tracks_distinct_unread_and_waiting_conversations() {
        let mut model = NotificationsModel::default();
        model.set_labels(labels());

        let first = model.on_signal(OwnerSignal::Completed {
            conversation_id: conv(1),
            title: "One".to_owned(),
            run_id: run(1),
        });
        assert_eq!(first.taskbar_badge_changed, Some(1));

        let second = model.on_signal(OwnerSignal::AwaitingAnswer {
            conversation_id: conv(2),
            title: "Two".to_owned(),
            part_id: part(1),
        });
        assert_eq!(second.taskbar_badge_changed, Some(2));

        let third = model.on_signal(OwnerSignal::Failed {
            conversation_id: conv(3),
            title: "Three".to_owned(),
            run_id: run(2),
        });
        assert_eq!(third.taskbar_badge_changed, Some(3));
        assert_eq!(model.taskbar_badge_count(), 3);

        // Starting a new run in conv(1) drops the count from 3 to 2.
        let restarted = model.on_signal(OwnerSignal::Active {
            conversation_id: conv(1),
        });
        assert_eq!(restarted.taskbar_badge_changed, Some(2));
        assert_eq!(model.taskbar_badge_count(), 2);

        // Stopping conv(2)'s question drops it to 1.
        let stopped = model.on_signal(OwnerSignal::EndedWithoutAlert {
            conversation_id: conv(2),
        });
        assert_eq!(stopped.taskbar_badge_changed, Some(1));

        // Deleting conv(3) clears it back to 0.
        let deleted = model.on_signal(OwnerSignal::Deleted {
            conversation_id: conv(3),
        });
        assert_eq!(deleted.taskbar_badge_changed, Some(0));
        assert_eq!(deleted.badge_changed, Some(false));
        assert_eq!(model.taskbar_badge_count(), 0);
    }

    #[test]
    fn starting_a_new_run_or_deleting_clears_unread_and_badge() {
        let mut model = NotificationsModel::default();
        model.set_labels(labels());
        model.on_signal(OwnerSignal::Completed {
            conversation_id: conv(1),
            title: "One".to_owned(),
            run_id: run(1),
        });
        model.on_signal(OwnerSignal::Completed {
            conversation_id: conv(2),
            title: "Two".to_owned(),
            run_id: run(2),
        });

        let restarted = model.on_signal(OwnerSignal::Active {
            conversation_id: conv(1),
        });
        assert_eq!(restarted.persist_clear, Some(conv(1)));
        assert_eq!(restarted.unread_changed, Some(vec![conv(2)]));
        assert!(model.badge_active());

        let deleted = model.on_signal(OwnerSignal::Deleted {
            conversation_id: conv(2),
        });
        assert_eq!(deleted.persist_clear, Some(conv(2)));
        assert_eq!(deleted.unread_changed, Some(Vec::new()));
        assert_eq!(deleted.badge_changed, Some(false));
    }
}
