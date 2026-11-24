// Pure topological SVG lane layout tests for the Commit Graph
// (plugins/git/docs/panel.md): linear chains, branching/merging, octopus
// merges, root commits, lane reuse and palette wrap-around.
import { describe, expect, it } from 'vitest';

import { GRAPH_PALETTE_SIZE, layoutCommitGraph } from '../../../ui/utils/git-graph.js';

describe('layoutCommitGraph', () => {
  it('keeps linear history on lane 0 with a single color', () => {
    const rows = layoutCommitGraph([
      { oid: 'c3', parents: ['c2'] },
      { oid: 'c2', parents: ['c1'] },
      { oid: 'c1', parents: [] },
    ]);

    expect(rows).toHaveLength(3);
    expect(rows.map((row) => row.nodeLane)).toEqual([0, 0, 0]);
    expect(rows.map((row) => row.nodeColor)).toEqual([0, 0, 0]);
    expect(rows.map((row) => row.laneCount)).toEqual([1, 1, 1]);
    expect(rows[0]?.edges).toEqual([{ color: 0, fromLane: 0, kind: 'outgoing', toLane: 0 }]);
    expect(rows[1]?.edges).toEqual([
      { color: 0, fromLane: 0, kind: 'incoming', toLane: 0 },
      { color: 0, fromLane: 0, kind: 'outgoing', toLane: 0 },
    ]);
    expect(rows[2]?.edges).toEqual([{ color: 0, fromLane: 0, kind: 'incoming', toLane: 0 }]);
  });

  it('opens a side lane for a two-parent merge and converges back at the merge base', () => {
    const rows = layoutCommitGraph([
      { oid: 'm1', parents: ['c2', 'f1'] },
      { oid: 'f1', parents: ['c1'] },
      { oid: 'c2', parents: ['c1'] },
      { oid: 'c1', parents: [] },
    ]);

    expect(rows[0]?.isMerge).toBe(true);
    expect(rows[0]?.nodeLane).toBe(0);
    expect(rows[0]?.laneCount).toBe(2);
    expect(rows[0]?.edges).toContainEqual({ color: 1, fromLane: 0, kind: 'outgoing', toLane: 1 });

    // Feature commit sits on lane 1 while lane 0 passes straight through toward c2.
    expect(rows[1]?.nodeLane).toBe(1);
    expect(rows[1]?.edges).toContainEqual({ color: 0, fromLane: 0, kind: 'pass', toLane: 0 });

    // c2 sits on lane 0 and connects straight to c1's active lane (lane 1) or merges cleanly.
    expect(rows[2]?.nodeLane).toBe(0);
    // Base commit c1 receives the converging edges cleanly.
    expect(rows[3]?.oid).toBe('c1');
    expect(rows[3]?.edges.some((edge) => edge.kind === 'incoming')).toBe(true);
  });

  it('allocates distinct lanes for octopus merges and shrinks trailing empty lanes', () => {
    const rows = layoutCommitGraph([
      { oid: 'octo', parents: ['p1', 'p2', 'p3'] },
      { oid: 'p3', parents: [] },
      { oid: 'p2', parents: [] },
      { oid: 'p1', parents: [] },
    ]);

    expect(rows[0]?.isMerge).toBe(true);
    expect(rows[0]?.laneCount).toBe(3);
    expect(rows[1]?.nodeLane).toBe(2);
    expect(rows[2]?.nodeLane).toBe(1);
    expect(rows[3]?.nodeLane).toBe(0);
    expect(rows[3]?.laneCount).toBe(1);
  });

  it('wraps palette indices cleanly within GRAPH_PALETTE_SIZE across many independent tips', () => {
    const commits = Array.from({ length: GRAPH_PALETTE_SIZE + 3 }, (_, idx) => ({
      oid: `tip-${idx}`,
      parents: [],
    }));
    const rows = layoutCommitGraph(commits);

    expect(
      rows.every((row) => {
        const isNonNegative = row.nodeColor >= 0;
        const isInPalette = row.nodeColor < GRAPH_PALETTE_SIZE;
        return isNonNegative && isInPalette;
      }),
    ).toBe(true);
    expect(rows[GRAPH_PALETTE_SIZE]?.nodeColor).toBe(0);
    // Disconnected root commits reuse lane 0 once closed.
    expect(rows.every((row) => row.nodeLane === 0)).toBe(true);
  });
});
