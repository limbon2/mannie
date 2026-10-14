<script lang="ts">
  import { markdownWorker } from '#lib/utils/markdown-parser.js';
  import { isCheapToParse, parseMarkdown, type RenderedMarkdown, StreamingMarkdown } from '#lib/utils/markdown.js';

  import MarkdownNodes from './MarkdownNodes.svelte';

  // Model output as Markdown (ADR-0021). A text within the cheap bounds is
  // parsed on the main thread; a longer or denser one in the Markdown worker,
  // which gives up after a time limit, so the window never waits for a parse.
  // While a reply streams, the text is parsed at most once per frame, and its
  // finished blocks only once.
  let { streaming = false, text }: { streaming?: boolean; text: string } = $props();

  const streamer = new StreamingMarkdown();
  const workerStream = markdownWorker.newStream();
  /** The latest result of a frame's parse or of the worker. */
  let latest = $state<RenderedMarkdown | null>(null);
  let frame = 0;
  /** Counts worker requests, so that an older answer never replaces a newer one. */
  let request = 0;
  let workerBusy = false;
  let workerWaiting: { readonly streaming: boolean; readonly text: string } | null = null;

  const rendered: RenderedMarkdown = $derived.by(() => {
    const parsesNow = !streaming && isCheapToParse(text);
    if (parsesNow) {
      return parseMarkdown(text);
    }
    return latest ?? { kind: 'plain', text };
  });

  function askWorker(current: string, isStreaming: boolean): void {
    if (workerBusy) {
      workerWaiting = { streaming: isStreaming, text: current };
      return;
    }
    workerBusy = true;
    request += 1;
    const asked = request;
    const answer = isStreaming ? markdownWorker.parseStream(workerStream, current) : markdownWorker.parse(current);
    void answer.then((result) => {
      workerBusy = false;
      if (asked === request) {
        latest = result;
      }
      const waiting = workerWaiting;
      workerWaiting = null;
      if (waiting !== null) {
        askWorker(waiting.text, waiting.streaming);
      }
    });
  }

  function parseFrame(): void {
    frame = 0;
    if (isCheapToParse(text)) {
      latest = streamer.update(text);
    } else {
      askWorker(text, true);
    }
  }

  $effect(() => {
    const current = text;
    if (streaming) {
      if (frame === 0) {
        frame = requestAnimationFrame(parseFrame);
      }
      return;
    }
    // The finished text is parsed whole, so a frame of the stream is not needed.
    if (frame !== 0) {
      cancelAnimationFrame(frame);
      frame = 0;
    }
    if (!isCheapToParse(current)) {
      askWorker(current, false);
    }
  });

  $effect(() => () => {
    if (frame !== 0) {
      cancelAnimationFrame(frame);
    }
    markdownWorker.forget(workerStream);
  });
</script>

<div class="text-sm/relaxed wrap-break-word">
  {#if rendered.kind === 'plain'}
    <p class="whitespace-pre-wrap" data-content>{rendered.text}</p>
  {:else}
    <MarkdownNodes nodes={rendered.nodes} />
  {/if}
</div>
