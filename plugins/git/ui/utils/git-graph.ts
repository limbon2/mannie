// Pure topological SVG lane layout engine for the Commit Graph.
// Given commits in topological order (`git log --topo-order`), assigns each
// commit a lane column, a palette index (0..7), and per-row vertical/curved
// edge segments so rows render independently with crisp branch and merge lines.

/** Minimal commit shape needed for lane layout. */
export interface GraphCommit {
  readonly oid: string;
  readonly parents: readonly string[];
}

/** A single edge segment drawn inside one commit row's SVG cell. */
export interface GraphEdge {
  readonly color: number;
  readonly fromLane: number;
  readonly kind: 'incoming' | 'outgoing' | 'pass';
  readonly toLane: number;
}

/** Per-row layout output for `CommitGraphSvg`. */
export interface GraphRow {
  readonly edges: readonly GraphEdge[];
  readonly isMerge: boolean;
  readonly laneCount: number;
  readonly nodeColor: number;
  readonly nodeLane: number;
  readonly oid: string;
}

/** Number of semantic graph lane colors (`--git-lane-0` .. `--git-lane-7`). */
export const GRAPH_PALETTE_SIZE = 8;

/** Allocates the lowest unused lane index, or appends a new lane. */
function allocateLane(lanes: (string | null)[], colors: number[], color: number, targetOid: string): number {
  const freeIndex = lanes.indexOf(null);
  if (freeIndex !== -1) {
    lanes[freeIndex] = targetOid;
    colors[freeIndex] = color;
    return freeIndex;
  }
  lanes.push(targetOid);
  colors.push(color);
  return lanes.length - 1;
}

/** Computes lane positions and per-row edge segments for an ordered commit list. */
export function layoutCommitGraph(commits: readonly GraphCommit[]): GraphRow[] {
  const lanes: (string | null)[] = [];
  const colors: number[] = [];
  const rows: GraphRow[] = [];
  let nextColor = 0;

  const takeColor = (): number => {
    const color = nextColor % GRAPH_PALETTE_SIZE;
    nextColor += 1;
    return color;
  };

  for (const commit of commits) {
    const edges: GraphEdge[] = [];
    let nodeLane = lanes.indexOf(commit.oid);
    let hasIncoming = nodeLane !== -1;

    if (nodeLane === -1) {
      nodeLane = allocateLane(lanes, colors, takeColor(), commit.oid);
      hasIncoming = false;
    }

    const nodeColor = colors[nodeLane] ?? 0;

    if (hasIncoming) {
      edges.push({
        color: nodeColor,
        fromLane: nodeLane,
        kind: 'incoming',
        toLane: nodeLane,
      });
    }

    // Any additional lanes expecting this same commit converge into `nodeLane`,
    // while every other active lane passes straight through this row.
    for (let lane = 0; lane < lanes.length; lane += 1) {
      const expected = lanes[lane];
      const isOwnLane = lane === nodeLane;
      const isEmpty = expected === null;
      if (isOwnLane || isEmpty) {
        continue;
      }
      const laneColor = colors[lane] ?? 0;
      if (expected === commit.oid) {
        edges.push({
          color: laneColor,
          fromLane: lane,
          kind: 'incoming',
          toLane: nodeLane,
        });
        lanes[lane] = null;
      } else {
        edges.push({
          color: laneColor,
          fromLane: lane,
          kind: 'pass',
          toLane: lane,
        });
      }
    }

    // Route outgoing edges to parents (first parent continues the lane when possible).
    const { parents } = commit;
    if (parents.length === 0) {
      lanes[nodeLane] = null;
    } else {
      const firstParent = parents[0];
      const existingFirst = lanes.findIndex((oid, idx) => {
        const isOtherLane = idx !== nodeLane;
        const matchesParent = oid === firstParent;
        return isOtherLane && matchesParent;
      });
      if (existingFirst === -1) {
        lanes[nodeLane] = firstParent;
        edges.push({
          color: nodeColor,
          fromLane: nodeLane,
          kind: 'outgoing',
          toLane: nodeLane,
        });
      } else {
        lanes[nodeLane] = null;
        edges.push({
          color: nodeColor,
          fromLane: nodeLane,
          kind: 'outgoing',
          toLane: existingFirst,
        });
      }

      for (let pIndex = 1; pIndex < parents.length; pIndex += 1) {
        const parentOid = parents[pIndex];
        const existing = lanes.indexOf(parentOid);
        if (existing === -1) {
          const branchColor = takeColor();
          const newLane = allocateLane(lanes, colors, branchColor, parentOid);
          edges.push({
            color: branchColor,
            fromLane: nodeLane,
            kind: 'outgoing',
            toLane: newLane,
          });
        } else {
          edges.push({
            color: colors[existing] ?? nodeColor,
            fromLane: nodeLane,
            kind: 'outgoing',
            toLane: existing,
          });
        }
      }
    }

    let maxLane = nodeLane;
    for (const edge of edges) {
      if (edge.fromLane > maxLane) {
        maxLane = edge.fromLane;
      }
      if (edge.toLane > maxLane) {
        maxLane = edge.toLane;
      }
    }

    rows.push({
      edges,
      isMerge: parents.length > 1,
      laneCount: maxLane + 1,
      nodeColor,
      nodeLane,
      oid: commit.oid,
    });

    // Shrink trailing empty lanes so subsequent rows stay narrow.
    while (lanes.length > 0) {
      if (lanes.at(-1) !== null) {
        break;
      }
      lanes.pop();
      colors.pop();
    }
  }

  return rows;
}
