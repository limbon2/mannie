// The words of the plugin's codes (ADR-0019): its errors, the refusals of
// files, the failures of sources and of searches, its work in progress and
// its start failure. A failure code of a source that it does not know gets a
// general sentence, and a code of a search that it does not know gets none,
// so that the core words it.
import { describe, expect, it } from 'vitest';

import type { NotebooksError } from '../../../ui/generated/bindings.js';
import type { NotebooksFailure } from '../../../ui/types/notebooks.js';
import {
  contextFailureMessage,
  contextFoundMessage,
  failureCode,
  notebooksFailureMessage,
  notebooksOffQuestion,
  notebooksStartFailure,
  sourceFailureMessage,
  sourceRefusalMessage,
} from '../../../ui/utils/notebooks-messages.js';

/** A call that the plugin answered with an error. */
function pluginFailure(error: NotebooksError): NotebooksFailure {
  return { error: { ...error, description: '', origin: 'plugin' }, kind: 'core' };
}

const HOST_REFUSAL: NotebooksFailure = {
  error: { code: 'plugin_off', description: '', origin: 'host', plugin: 'notebooks' },
  kind: 'core',
};

describe('the messages of the plugin', () => {
  it('word each error of the plugin with a sentence of its own', () => {
    const errors: NotebooksError[] = [
      { code: 'dialog_failed' },
      { code: 'name_taken' },
      { code: 'notebook_not_found' },
      { code: 'nothing_to_retry' },
      { code: 'passage_not_found' },
      { code: 'profile_not_found' },
      { code: 'profiles_unavailable' },
      { code: 'refused', reason: 'duplicate' },
      { code: 'source_not_found' },
      { code: 'storage_failed' },
      { code: 'upload_not_found' },
    ];

    const sentences = errors.map((error) => notebooksFailureMessage(pluginFailure(error)));

    expect(new Set(sentences).size).toBe(errors.length);
    expect(notebooksFailureMessage(pluginFailure({ code: 'upload_part_out_of_order', expected: 2 }))).toBe(
      "The file's upload broke off. Drop the file again.",
    );
    expect(notebooksFailureMessage(pluginFailure({ code: 'refused', reason: 'duplicate' }))).toBe(
      'The notebook holds this file already.',
    );
  });

  it("leave the host's refusals and calls without an answer to the core's words", () => {
    expect(notebooksFailureMessage(HOST_REFUSAL)).toContain('Notebooks');
    expect(notebooksFailureMessage({ kind: 'ipc' })).not.toBe('');
    expect(failureCode(HOST_REFUSAL)).toBeNull();
    expect(failureCode({ kind: 'ipc' })).toBeNull();
    expect(failureCode(pluginFailure({ code: 'source_not_found' }))).toBe('source_not_found');
  });

  it('word each refusal of a file', () => {
    expect(sourceRefusalMessage('duplicate')).toBe('The notebook holds this file already.');
    expect(sourceRefusalMessage('unsupported_type')).toBe('Only PDF, Markdown and text files can be added.');
    expect(sourceRefusalMessage('unreadable')).toBe('The file could not be read.');
  });

  it('word the failures of sources, and a code that it does not know in general', () => {
    expect(sourceFailureMessage('no_text')).toBe('This PDF has no text. It may be a scan.');
    // A source of the first version that had too many pages reads as one that failed.
    expect(sourceFailureMessage('too_many_pages')).toBe('The source could not be indexed.');
    expect(sourceFailureMessage('server_unreachable')).toBe('The embedding server could not be reached.');
    expect(sourceFailureMessage('interrupted')).toContain('add the file again');
    expect(sourceFailureMessage('a_later_code')).toBe('The source could not be indexed.');
  });

  it('word the failures of searches, and leave a code that it does not know to the core', () => {
    expect(contextFailureMessage('notebook_not_found')).toBe('The notebook was deleted.');
    expect(contextFailureMessage('nothing_indexed')).toBe('None of its sources is ready and used in answers.');
    expect(contextFailureMessage('a_later_code')).toBeNull();
  });

  it('name what a search found in the language of its sentence', () => {
    expect(contextFoundMessage(['Physics'], 1)).toBe('Searched Physics: 1 passage');
    expect(contextFoundMessage(['Physics', 'Optics'], 5)).toBe('Searched Physics and Optics: 5 passages');
  });

  it('ask before turning off only while sources are read or indexed', () => {
    expect(notebooksOffQuestion([])).toBeNull();
    expect(notebooksOffQuestion([{ code: 'indexing_sources', count: 1 }])).toBe(
      'A source is being read or indexed. Its indexing continues when Notebooks is turned on again.',
    );
    expect(notebooksOffQuestion([{ code: 'indexing_sources', count: 3 }])).toContain('3 sources');
  });

  it('word its start failure, and no code that it does not know', () => {
    expect(notebooksStartFailure('storage_failed')).toContain('could not read its notebooks');
    expect(notebooksStartFailure('other')).toBeNull();
  });
});
