<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as RadioGroup from '@mannie/plugin-sdk/components/radio-group';
  import * as Select from '@mannie/plugin-sdk/components/select';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import type { Renderer, TerminalSettings } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { MAX_FONT_SIZE, MAX_SCROLLBACK, MIN_FONT_SIZE, MIN_SCROLLBACK } from '../generated/plugin.js';
  import { terminals } from '../state/terminals.svelte.js';
  import { terminalFailureMessage } from '../utils/terminal-messages.js';

  // The Terminal section of the settings (plugins/terminal/docs/README.md,
  // settings): the default shell of this computer, the lines of scrollback,
  // the font size, whether the cursor blinks, how terminals draw and whether
  // they serve screen readers. A change goes to the plugin at once; new
  // views use it, and open ones when they show again.
  const settings = $derived(terminals.settings);
  const shells = $derived(terminals.hosts?.shells ?? []);
  const SYSTEM_DEFAULT = '';

  async function update(change: Partial<TerminalSettings>): Promise<void> {
    await terminals.updateSettings({ ...settings, ...change });
  }

  /** A whole number from `minimum` to `maximum` that a field holds, or `null`. */
  function numberIn(value: string, minimum: number, maximum: number): number | null {
    const number = Number(value);
    if (!Number.isInteger(number)) {
      return null;
    }
    const isInRange = number >= minimum && number <= maximum;
    return isInRange ? number : null;
  }

  function renderer(value: string): Renderer {
    switch (value) {
      case 'compatible':
      case 'gpu': {
        return value;
      }
      default: {
        return 'automatic';
      }
    }
  }

  function changeShell(value: string): void {
    void update({ defaultShell: value === SYSTEM_DEFAULT ? null : value });
  }

  function changeScrollback(event: Event & { currentTarget: HTMLInputElement }): void {
    const lines = numberIn(event.currentTarget.value, MIN_SCROLLBACK, MAX_SCROLLBACK);
    if (lines !== null) {
      void update({ scrollback: lines });
    }
  }

  function changeFontSize(event: Event & { currentTarget: HTMLInputElement }): void {
    const size = numberIn(event.currentTarget.value, MIN_FONT_SIZE, MAX_FONT_SIZE);
    if (size !== null) {
      void update({ fontSize: size });
    }
  }

  function changeRenderer(value: string): void {
    void update({ renderer: renderer(value) });
  }

  function changeCursorBlink(checked: boolean): void {
    void update({ cursorBlink: checked });
  }

  function changeScreenReader(checked: boolean): void {
    void update({ screenReader: checked });
  }
</script>

<div class="flex flex-col gap-6">
  {#if terminals.settingsFailure !== null}
    <Alert.Root variant="destructive">
      <CircleAlertIcon />
      <Alert.Description>{terminalFailureMessage(terminals.settingsFailure)}</Alert.Description>
    </Alert.Root>
  {/if}
  <Field.Field>
    <Field.Label for="terminal-default-shell">{m.terminal_settings_default_shell()}</Field.Label>
    <Select.Root type="single" value={settings.defaultShell ?? SYSTEM_DEFAULT} onValueChange={changeShell}>
      <Select.Trigger id="terminal-default-shell" class="w-72">
        {shells.find((shell) => shell.program === settings.defaultShell)?.name ?? m.terminal_settings_system_default()}
      </Select.Trigger>
      <Select.Content>
        <Select.Item value={SYSTEM_DEFAULT}>{m.terminal_settings_system_default()}</Select.Item>
        {#each shells as shell (shell.program)}
          <Select.Item value={shell.program}>{shell.name}</Select.Item>
        {/each}
      </Select.Content>
    </Select.Root>
    <Field.Description>{m.terminal_settings_default_shell_description()}</Field.Description>
  </Field.Field>
  <Field.Field>
    <Field.Label for="terminal-scrollback">{m.terminal_settings_scrollback()}</Field.Label>
    <Input
      id="terminal-scrollback"
      class="w-40"
      inputmode="numeric"
      max={MAX_SCROLLBACK}
      min={MIN_SCROLLBACK}
      type="number"
      value={settings.scrollback}
      onchange={changeScrollback}
    />
    <Field.Description>
      {m.terminal_settings_scrollback_description({ max: MAX_SCROLLBACK, min: MIN_SCROLLBACK })}
    </Field.Description>
  </Field.Field>
  <Field.Field>
    <Field.Label for="terminal-font-size">{m.terminal_settings_font_size()}</Field.Label>
    <Input
      id="terminal-font-size"
      class="w-40"
      inputmode="numeric"
      max={MAX_FONT_SIZE}
      min={MIN_FONT_SIZE}
      type="number"
      value={settings.fontSize}
      onchange={changeFontSize}
    />
  </Field.Field>
  <Field.Field orientation="horizontal">
    <Field.Content>
      <Field.Label for="terminal-cursor-blink">{m.terminal_settings_cursor_blink()}</Field.Label>
      <Field.Description>{m.terminal_settings_cursor_blink_description()}</Field.Description>
    </Field.Content>
    <Switch id="terminal-cursor-blink" checked={settings.cursorBlink} onCheckedChange={changeCursorBlink} />
  </Field.Field>
  <Field.Set>
    <Field.Legend variant="label">{m.terminal_settings_renderer()}</Field.Legend>
    <RadioGroup.Root value={settings.renderer} onValueChange={changeRenderer}>
      <Field.Field orientation="horizontal">
        <RadioGroup.Item id="terminal-renderer-automatic" value="automatic" />
        <Field.Label class="font-normal" for="terminal-renderer-automatic"
          >{m.terminal_settings_renderer_automatic()}</Field.Label
        >
      </Field.Field>
      <Field.Field orientation="horizontal">
        <RadioGroup.Item id="terminal-renderer-gpu" value="gpu" />
        <Field.Label class="font-normal" for="terminal-renderer-gpu">{m.terminal_settings_renderer_gpu()}</Field.Label>
      </Field.Field>
      <Field.Field orientation="horizontal">
        <RadioGroup.Item id="terminal-renderer-compatible" value="compatible" />
        <Field.Label class="font-normal" for="terminal-renderer-compatible">
          {m.terminal_settings_renderer_compatible()}
        </Field.Label>
      </Field.Field>
    </RadioGroup.Root>
  </Field.Set>
  <Field.Field orientation="horizontal">
    <Field.Content>
      <Field.Label for="terminal-screen-reader">{m.terminal_settings_screen_reader()}</Field.Label>
      <Field.Description>{m.terminal_settings_screen_reader_description()}</Field.Description>
    </Field.Content>
    <Switch id="terminal-screen-reader" checked={settings.screenReader} onCheckedChange={changeScreenReader} />
  </Field.Field>
</div>
