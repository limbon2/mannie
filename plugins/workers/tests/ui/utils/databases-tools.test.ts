// The lines of the cards of Databases' tools (ADR-0039; plugins/workers/
// docs/databases.md, agents): a call names its database while it runs, and
// what it did once it ended, from the data that the plugin returned.
import type { ToolKindCall } from '@mannie/plugin-sdk/slots';
import { describe, expect, it } from 'vitest';

import { changeSummary, describeSummary, querySummary } from '../../../ui/utils/databases-tools.js';

function call(fields: Partial<Pick<ToolKindCall, 'arguments' | 'data'>>): ToolKindCall {
  return { arguments: fields.arguments ?? {}, data: fields.data ?? null, failure: null, status: 'completed' };
}

describe('the lines of the database tools', () => {
  it('name the database of a running call from its arguments', () => {
    const running = call({ arguments: { database: 'Inventory' } });

    expect(describeSummary(running)).toContain('Inventory');
    expect(querySummary(running)).toContain('Inventory');
    expect(changeSummary(running)).toContain('Inventory');
  });

  it('say what an ended call did from its data', () => {
    const queried = querySummary(call({ data: { database: 'Inventory', rows: 3 } }));
    const changed = changeSummary(call({ data: { changes: [2, 1], database: 'Inventory' } }));
    const described = describeSummary(call({ data: { database: 'Inventory', tables: 4 } }));

    expect(queried).toMatch(/3/u);
    expect(changed).toMatch(/3/u);
    expect(described).toMatch(/4/u);
    expect(queried).not.toBe(querySummary(call({ arguments: { database: 'Inventory' } })));
  });

  it('give no line for a call that names no database', () => {
    expect(querySummary(call({}))).toBeNull();
    expect(changeSummary(call({ data: { changes: 'many' } }))).toBeNull();
  });
});
