<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
  import { Switch } from '#lib/components/ui/switch/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import { toolsFailureMessage } from '#lib/utils/tools-messages.js';

  // Allows a command and saves a rule from it (docs/features/system-tools/
  // tool-activity.md, approval requests): the owner chooses how many of the
  // command's leading words the rule keeps, and whether it applies only in
  // the command's working directory. A rule from a command on a remote host
  // always applies only on that host (ADR-0036), so the dialog names the
  // host instead. The core builds the rule from these answers and the request
  // it answers. `toolsActions.startSaveRule` opens it.
  interface PrefixChoice {
    readonly count: number;
    readonly text: string;
  }

  const request = $derived(toolsActions.rule);
  const remote = $derived(request?.proposal.remote ?? null);
  const deciding = $derived(request !== null && toolsActions.deciding.includes(request.partId));
  const failure = $derived(request === null ? null : (toolsActions.decisionFailures[request.partId] ?? null));
  const choices: PrefixChoice[] = $derived(
    (request?.proposal.words ?? []).map((_word, index, words) => ({
      count: index + 1,
      text: words.slice(0, index + 1).join(' '),
    })),
  );

  function choosePrefix(value: string): void {
    if (request === null) {
      return;
    }
    const count = Number(value);
    if (Number.isInteger(count)) {
      request.prefixWords = count;
    }
  }

  function save(event: SubmitEvent): void {
    event.preventDefault();
    if (request !== null) {
      void toolsActions.saveRule(request);
    }
  }
</script>

<Dialog.Root bind:open={toolsActions.ruleOpen}>
  <Dialog.Content class="sm:max-w-xl" showCloseButton={false}>
    {#if request !== null}
      <form class="flex flex-col gap-4" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title>{m.tools_rule_title()}</Dialog.Title>
          <Dialog.Description>
            {remote === null ? m.tools_rule_description() : m.tools_rule_remote_description()}
          </Dialog.Description>
        </Dialog.Header>
        <Field.Set>
          <Field.Legend variant="label">{m.tools_rule_words_legend()}</Field.Legend>
          <RadioGroup.Root
            class="max-h-60 overflow-y-auto pe-1"
            value={String(request.prefixWords)}
            onValueChange={choosePrefix}
          >
            {#each choices as choice (choice.count)}
              <Field.Field orientation="horizontal">
                <RadioGroup.Item id="tools-rule-words-{choice.count}" value={String(choice.count)} />
                <Field.Label class="font-normal" for="tools-rule-words-{choice.count}">
                  <span class="font-mono break-all" data-content>{choice.text}</span>
                </Field.Label>
              </Field.Field>
            {/each}
          </RadioGroup.Root>
        </Field.Set>
        {#if remote === null}
          <Field.Field orientation="horizontal">
            <Field.Content>
              <Field.Label for="tools-rule-folder">{m.tools_rule_limit_folder()}</Field.Label>
              <Field.Description>
                <span class="font-mono break-all" data-content>{request.proposal.folder}</span>
              </Field.Description>
            </Field.Content>
            <Switch id="tools-rule-folder" bind:checked={request.limitToFolder} />
          </Field.Field>
        {:else}
          <p class="text-sm" data-rule-host="">
            {m.tools_rule_limit_host({ host: remote.label })}
            <span class="block font-mono text-xs break-all text-muted-foreground" data-content>{remote.endpoint}</span>
          </p>
        {/if}
        {#if failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Description>{toolsFailureMessage(failure)}</Alert.Description>
          </Alert.Root>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (toolsActions.ruleOpen = false)}>
            {m.tools_cancel()}
          </Button>
          <Button disabled={deciding} type="submit">{m.tools_rule_save()}</Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
