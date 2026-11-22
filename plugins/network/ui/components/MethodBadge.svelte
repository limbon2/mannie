<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';

  interface Props {
    class?: string;
    method: string;
  }

  let { class: className = '', method }: Props = $props();

  const tone = $derived.by(() => {
    switch (method.toUpperCase()) {
      case 'GET': {
        return 'border-emerald-600/35 bg-emerald-500/10 text-emerald-900 dark:text-emerald-300';
      }
      case 'POST': {
        return 'border-amber-600/35 bg-amber-500/10 text-amber-900 dark:text-amber-300';
      }
      case 'PUT': {
        return 'border-sky-600/35 bg-sky-500/10 text-sky-900 dark:text-sky-300';
      }
      case 'PATCH': {
        return 'border-violet-600/35 bg-violet-500/10 text-violet-900 dark:text-violet-300';
      }
      case 'DELETE': {
        return 'border-destructive/35 bg-destructive/10 text-destructive';
      }
      default: {
        return 'border-border bg-muted text-muted-foreground';
      }
    }
  });
</script>

<Badge
  class="shrink-0 rounded-sm font-mono text-[10px] font-semibold tracking-wide uppercase {tone} {className}"
  data-content=""
  variant="outline"
>
  {method.toUpperCase()}
</Badge>
