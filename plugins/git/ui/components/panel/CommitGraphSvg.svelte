<script lang="ts">
  import type { GraphRow } from '../../utils/git-graph.js';

  // Pure SVG renderer for one commit row's lane lines, Bézier merge/branch
  // curves, and commit node dot. Uses Tailwind theme stroke/fill classes and
  // SVG presentation attributes only (zero inline `style` attributes for CSP).
  interface Props {
    readonly isHead?: boolean;
    readonly row: GraphRow;
  }

  let { isHead = false, row }: Props = $props();

  const LANE_SPACING = 14;
  const ROW_HEIGHT = 28;
  const HALF_Y = ROW_HEIGHT / 2;
  const PAD_X = 8;

  const STROKE_CLASSES: readonly string[] = [
    'stroke-git-lane-0',
    'stroke-git-lane-1',
    'stroke-git-lane-2',
    'stroke-git-lane-3',
    'stroke-git-lane-4',
    'stroke-git-lane-5',
    'stroke-git-lane-6',
    'stroke-git-lane-7',
  ];

  const FILL_CLASSES: readonly string[] = [
    'fill-git-lane-0',
    'fill-git-lane-1',
    'fill-git-lane-2',
    'fill-git-lane-3',
    'fill-git-lane-4',
    'fill-git-lane-5',
    'fill-git-lane-6',
    'fill-git-lane-7',
  ];

  const width = $derived(Math.max(24, PAD_X * 2 + Math.max(0, row.laneCount - 1) * LANE_SPACING));
  const nodeX = $derived(PAD_X + row.nodeLane * LANE_SPACING);

  function laneX(lane: number): number {
    return PAD_X + lane * LANE_SPACING;
  }

  function strokeClass(color: number): string {
    return STROKE_CLASSES[color % STROKE_CLASSES.length] ?? 'stroke-git-lane-0';
  }

  function fillClass(color: number): string {
    return FILL_CLASSES[color % FILL_CLASSES.length] ?? 'fill-git-lane-0';
  }

  function edgePath(kind: 'incoming' | 'outgoing' | 'pass', fromLane: number, toLane: number): string {
    const x1 = laneX(fromLane);
    const x2 = laneX(toLane);
    if (kind === 'pass') {
      return `M ${x1} 0 L ${x1} ${ROW_HEIGHT}`;
    }
    if (kind === 'incoming') {
      if (x1 === x2) {
        return `M ${x1} 0 L ${x2} ${HALF_Y}`;
      }
      return `M ${x1} 0 C ${x1} ${HALF_Y * 0.65}, ${x2} ${HALF_Y * 0.35}, ${x2} ${HALF_Y}`;
    }
    if (x1 === x2) {
      return `M ${x1} ${HALF_Y} L ${x2} ${ROW_HEIGHT}`;
    }
    return `M ${x1} ${HALF_Y} C ${x1} ${HALF_Y + HALF_Y * 0.65}, ${x2} ${HALF_Y + HALF_Y * 0.35}, ${x2} ${ROW_HEIGHT}`;
  }
</script>

<svg
  class="shrink-0 overflow-visible"
  aria-hidden="true"
  height={ROW_HEIGHT}
  viewBox={`0 0 ${width} ${ROW_HEIGHT}`}
  {width}
>
  {#each row.edges as edge, index (`${edge.kind}-${edge.fromLane}-${edge.toLane}-${index}`)}
    <path
      class={strokeClass(edge.color)}
      d={edgePath(edge.kind, edge.fromLane, edge.toLane)}
      fill="none"
      stroke-linecap="round"
      stroke-width="1.75"
    />
  {/each}

  {#if isHead}
    <circle class={strokeClass(row.nodeColor)} cx={nodeX} cy={HALF_Y} fill="none" r="5.5" stroke-width="1.5" />
  {/if}

  {#if row.isMerge}
    <circle class={['fill-background', strokeClass(row.nodeColor)]} cx={nodeX} cy={HALF_Y} r="3.75" stroke-width="2" />
  {:else}
    <circle class={fillClass(row.nodeColor)} cx={nodeX} cy={HALF_Y} r="3.5" />
  {/if}
</svg>
