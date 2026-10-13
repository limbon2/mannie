<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import { resetMode, setMode, userPrefersMode } from 'mode-watcher';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
  import * as Select from '#lib/components/ui/select/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { interfaceSettings } from '#lib/state/interface-settings.svelte.js';
  import type { Message } from '#lib/types/shell-slots.js';
  import { settingsFailureMessage } from '#lib/utils/error-messages.js';
  import { availableLocales, languageName, systemLocale } from '#lib/utils/languages.js';

  interface LanguageItem {
    readonly label: string;
    readonly value: string;
  }

  interface ThemeChoice {
    readonly label: Message;
    readonly value: 'dark' | 'light' | 'system';
  }

  /** The select's value for "follow the system". No language tag can be `*`. */
  const SYSTEM_LANGUAGE = '*';

  const THEMES: readonly ThemeChoice[] = [
    { label: m.shell_settings_theme_light, value: 'light' },
    { label: m.shell_settings_theme_dark, value: 'dark' },
    { label: m.shell_settings_theme_system, value: 'system' },
  ];

  const languages = $derived(languageItems());
  const selectedLanguage = $derived(interfaceSettings.chosenLocale ?? SYSTEM_LANGUAGE);
  const languageLocked = $derived(interfaceSettings.saving || interfaceSettings.readFailure !== null);

  function languageItems(): LanguageItem[] {
    const system = {
      label: m.shell_settings_language_system({ language: languageName(systemLocale()) }),
      value: SYSTEM_LANGUAGE,
    };
    const catalogs = availableLocales().map((locale) => ({
      label: m.shell_settings_language_option({ language: languageName(locale) }),
      value: locale,
    }));
    return [system, ...catalogs];
  }

  function changeLanguage(value: string): void {
    if (value === selectedLanguage) {
      return;
    }
    void interfaceSettings.changeLocale(value === SYSTEM_LANGUAGE ? null : value);
  }

  function changeTheme(value: string): void {
    if (value === 'system') {
      resetMode();
      return;
    }
    const isMode = value === 'light' || value === 'dark';
    if (isMode) {
      setMode(value);
    }
  }
</script>

{#if interfaceSettings.readFailure !== null}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.shell_settings_load_failed_title()}</Alert.Title>
    <Alert.Description>{settingsFailureMessage(interfaceSettings.readFailure)}</Alert.Description>
  </Alert.Root>
{/if}
<Field.Group>
  <Field.Field>
    <Field.Label for="interface-language">{m.shell_settings_language_label()}</Field.Label>
    <Select.Root
      disabled={languageLocked}
      items={languages}
      type="single"
      value={selectedLanguage}
      onValueChange={changeLanguage}
    >
      <Select.Trigger id="interface-language" class="w-full max-w-xs">
        <Select.Value />
      </Select.Trigger>
      <Select.Content>
        {#each languages as language (language.value)}
          <Select.Item label={language.label} value={language.value}>{language.label}</Select.Item>
        {/each}
      </Select.Content>
    </Select.Root>
    <Field.Description>{m.shell_settings_language_description()}</Field.Description>
    {#if interfaceSettings.saving}
      <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
        <Spinner aria-label={m.shell_loading()} />
        <span>{m.shell_settings_language_saving()}</span>
      </p>
    {/if}
    {#if interfaceSettings.saveFailure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.shell_settings_save_failed_title()}</Alert.Title>
        <Alert.Description>{settingsFailureMessage(interfaceSettings.saveFailure)}</Alert.Description>
      </Alert.Root>
    {/if}
  </Field.Field>
  <Field.Set>
    <Field.Legend variant="label">{m.shell_settings_theme_label()}</Field.Legend>
    <RadioGroup.Root value={userPrefersMode.current} onValueChange={changeTheme}>
      {#each THEMES as theme (theme.value)}
        <Field.Field orientation="horizontal">
          <RadioGroup.Item id="interface-theme-{theme.value}" value={theme.value} />
          <Field.Label class="font-normal" for="interface-theme-{theme.value}">{theme.label()}</Field.Label>
        </Field.Field>
      {/each}
    </RadioGroup.Root>
  </Field.Set>
</Field.Group>
