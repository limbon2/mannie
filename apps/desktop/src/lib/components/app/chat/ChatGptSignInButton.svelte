<script lang="ts">
  import logoOnLight from '#lib/assets/chatgpt/chatgpt-logo-black.svg';
  import logoOnDark from '#lib/assets/chatgpt/chatgpt-logo-white.svg';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';

  // OpenAI's button for signing in with ChatGPT, in one of the formats that
  // OpenAI's guidelines approve (src/lib/assets/chatgpt/SOURCE.md; ADR-0062):
  // the ChatGPT logo, then "Continue with ChatGPT". The outline button has the
  // page's background, so the black logo shows on the light theme and the
  // white one on the dark theme. The logo is decorative, and the button takes
  // its name from its text. OpenAI asks for `data-oai-siwc-button` on every
  // such button.
  interface Props {
    disabled?: boolean;
    onclick: () => void;
    size?: 'default' | 'sm';
  }

  let { disabled = false, onclick, size = 'default' }: Props = $props();
</script>

<Button data-chatgpt-sign-in="" data-oai-siwc-button="" {disabled} {size} variant="outline" {onclick}>
  <img class="size-4 shrink-0 dark:hidden" alt="" src={logoOnLight} />
  <img class="hidden size-4 shrink-0 dark:block" alt="" src={logoOnDark} />
  {m.chat_chatgpt_sign_in()}
</Button>
