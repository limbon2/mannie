<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { onMount } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Separator } from '#lib/components/ui/separator/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { Switch } from '#lib/components/ui/switch/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { notificationSettings, type NotificationToggleKey } from '#lib/state/notification-settings.svelte.js';
  import type { Message } from '#lib/types/shell-slots.js';
  import { notificationFailureMessage } from '#lib/utils/notifications-messages.js';

  // Settings -> Notifications (docs/features/notifications.md; ADR-0074):
  // master desktop notification switch, per-event toggles and the independent
  // system tray badge toggle.
  interface EventSwitchItem {
    readonly description: Message;
    readonly id: string;
    readonly key: Exclude<NotificationToggleKey, 'enabled' | 'taskbarBadge' | 'trayBadge'>;
    readonly label: Message;
  }

  const EVENT_SWITCHES: readonly EventSwitchItem[] = [
    {
      description: m.notifications_settings_question_description,
      id: 'notifications-on-question',
      key: 'onQuestion',
      label: m.notifications_settings_question_label,
    },
    {
      description: m.notifications_settings_approval_description,
      id: 'notifications-on-approval',
      key: 'onApproval',
      label: m.notifications_settings_approval_label,
    },
    {
      description: m.notifications_settings_completion_description,
      id: 'notifications-on-completion',
      key: 'onCompletion',
      label: m.notifications_settings_completion_label,
    },
    {
      description: m.notifications_settings_failure_description,
      id: 'notifications-on-failure',
      key: 'onFailure',
      label: m.notifications_settings_failure_label,
    },
  ];

  const settings = $derived(notificationSettings.settings);

  function toggle(key: NotificationToggleKey, checked: boolean): void {
    void notificationSettings.setToggle(key, checked);
  }

  function reload(): void {
    void notificationSettings.load();
  }

  onMount(() => {
    void notificationSettings.load();
  });
</script>

<div class="flex flex-col gap-6" data-notifications-section="">
  {#if notificationSettings.readFailure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Title>{m.notifications_settings_load_failed_title()}</Alert.Title>
      <Alert.Description>{notificationFailureMessage(notificationSettings.readFailure)}</Alert.Description>
      <Alert.Action>
        <Button size="sm" variant="outline" onclick={reload}>{m.notifications_try_again()}</Button>
      </Alert.Action>
    </Alert.Root>
  {:else if settings === null}
    <div class="flex flex-col gap-3" aria-busy="true">
      <Skeleton class="h-5 w-48" />
      <Skeleton class="h-5 w-64" />
      <Skeleton class="h-5 w-56" />
    </div>
  {:else}
    <Field.Set>
      <Field.Legend variant="label">{m.notifications_settings_desktop_legend()}</Field.Legend>
      <Field.Description>{m.notifications_settings_desktop_description()}</Field.Description>
      <div class="flex flex-col gap-4">
        <Field.Field orientation="horizontal">
          <Field.Content>
            <Field.Label for="notifications-enabled">{m.notifications_settings_enabled_label()}</Field.Label>
            <Field.Description id="notifications-enabled-help">
              {m.notifications_settings_enabled_description()}
            </Field.Description>
          </Field.Content>
          <Switch
            id="notifications-enabled"
            aria-describedby="notifications-enabled-help"
            disabled={notificationSettings.saving}
            bind:checked={() => settings.enabled, (checked: boolean) => toggle('enabled', checked)}
          />
        </Field.Field>

        {#each EVENT_SWITCHES as item (item.key)}
          <Field.Field orientation="horizontal">
            <Field.Content>
              <Field.Label class="font-normal" for={item.id}>{item.label()}</Field.Label>
              <Field.Description id="{item.id}-help">{item.description()}</Field.Description>
            </Field.Content>
            <Switch
              id={item.id}
              aria-describedby="{item.id}-help"
              disabled={notificationSettings.saving || !settings.enabled}
              bind:checked={() => settings[item.key], (checked: boolean) => toggle(item.key, checked)}
            />
          </Field.Field>
        {/each}
      </div>
    </Field.Set>

    <Separator />

    <Field.Set>
      <Field.Legend variant="label">{m.notifications_settings_badges_legend()}</Field.Legend>
      <div class="flex flex-col gap-4">
        <Field.Field orientation="horizontal">
          <Field.Content>
            <Field.Label for="notifications-taskbar-badge">
              {m.notifications_settings_taskbar_badge_label()}
            </Field.Label>
            <Field.Description id="notifications-taskbar-badge-help">
              {m.notifications_settings_taskbar_badge_description()}
            </Field.Description>
          </Field.Content>
          <Switch
            id="notifications-taskbar-badge"
            aria-describedby="notifications-taskbar-badge-help"
            disabled={notificationSettings.saving}
            bind:checked={() => settings.taskbarBadge, (checked: boolean) => toggle('taskbarBadge', checked)}
          />
        </Field.Field>

        <Field.Field orientation="horizontal">
          <Field.Content>
            <Field.Label for="notifications-tray-badge">{m.notifications_settings_tray_badge_label()}</Field.Label>
            <Field.Description id="notifications-tray-badge-help">
              {m.notifications_settings_tray_badge_description()}
            </Field.Description>
          </Field.Content>
          <Switch
            id="notifications-tray-badge"
            aria-describedby="notifications-tray-badge-help"
            disabled={notificationSettings.saving}
            bind:checked={() => settings.trayBadge, (checked: boolean) => toggle('trayBadge', checked)}
          />
        </Field.Field>
      </div>
    </Field.Set>

    {#if notificationSettings.saving}
      <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
        <Spinner aria-label={m.notifications_settings_saving()} />
        <span>{m.notifications_settings_saving()}</span>
      </p>
    {/if}

    {#if notificationSettings.saveFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.notifications_settings_save_failed_title()}</Alert.Title>
        <Alert.Description>{notificationFailureMessage(notificationSettings.saveFailure)}</Alert.Description>
      </Alert.Root>
    {/if}
  {/if}
</div>
