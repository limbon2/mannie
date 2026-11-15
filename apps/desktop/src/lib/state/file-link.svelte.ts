// Local navigation is an owner action. Only Rust resolves or opens a path.
import { commands, type FileLinkError, type FileLinkRef, type ResolvedFileLink } from '#lib/generated/bindings.js';

export type FileLinkFailure = FileLinkError | { readonly code: 'ipc' };

/** State for one rendered reference, including its context menu. */
export class FileLinkState {
  resolved: ResolvedFileLink | null = $state(null);
  failure: FileLinkFailure | null = $state(null);
  pending: 'checking' | 'opening' | null = $state(null);
  #request = 0;

  readonly link: FileLinkRef;

  constructor(link: FileLinkRef) {
    this.link = link;
  }

  /** Invalidates answers from a previous path, conversation or component. */
  reset(): void {
    this.#request += 1;
    this.resolved = null;
    this.failure = null;
    this.pending = null;
  }

  async resolve(): Promise<void> {
    if (this.pending !== null) {
      return;
    }
    const request = ++this.#request;
    this.pending = 'checking';
    this.failure = null;
    this.resolved = null;
    try {
      const result = await commands.resolveFileLink(this.link);
      if (request !== this.#request) {
        return;
      }
      if (result.status === 'ok') {
        this.resolved = result.data;
      } else {
        this.failure = result.error;
      }
    } catch {
      if (request === this.#request) {
        this.failure = { code: 'ipc' };
      }
    } finally {
      if (request === this.#request) {
        this.pending = null;
      }
    }
  }

  async open(): Promise<void> {
    if (this.pending !== null) {
      return;
    }
    const request = ++this.#request;
    this.pending = 'opening';
    this.failure = null;
    try {
      const result = await commands.openFileLink({ link: this.link, target: 'file_manager' });
      const current = request === this.#request;
      const failed = result.status === 'error';
      if (current && failed) {
        this.failure = result.error;
      }
    } catch {
      if (request === this.#request) {
        this.failure = { code: 'ipc' };
      }
    } finally {
      if (request === this.#request) {
        this.pending = null;
      }
    }
  }
}
