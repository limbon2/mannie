// The link that waits for the owner's confirmation (ADR-0021, decision 4):
// the core checks it first, the owner sees it as the core normalized it, and
// it opens in the system browser only after the owner confirms, through the
// core, which checks it again.
import { type CheckedLink, commands, type LinkError } from '#lib/generated/bindings.js';

/** Why a link could not be checked or opened: an error the core reported, or no answer at all. */
export type LinkFailure = { readonly error: LinkError; readonly kind: 'core' } | { readonly kind: 'ipc' };

/** Where the confirmation of a link stands. */
export type LinkConfirmationState =
  | { readonly failure: LinkFailure; readonly kind: 'refused' }
  | {
      readonly failure: LinkFailure | null;
      readonly kind: 'confirm';
      readonly link: CheckedLink;
      readonly opening: boolean;
    }
  | { readonly kind: 'checking' }
  | { readonly kind: 'closed' };

/** The confirmation of links from model output. One link waits at a time. */
export class LinkConfirmation {
  state: LinkConfirmationState = $state({ kind: 'closed' });
  /** Counts requests, so that the answer to an older one changes nothing. */
  #request = 0;

  /** Asks the core to check a link, then asks the owner to confirm it. */
  async request(url: string): Promise<void> {
    this.#request += 1;
    const request = this.#request;
    this.state = { kind: 'checking' };
    let next: LinkConfirmationState;
    try {
      const result = await commands.checkLink({ url });
      next =
        result.status === 'ok'
          ? { failure: null, kind: 'confirm', link: result.data, opening: false }
          : { failure: { error: result.error, kind: 'core' }, kind: 'refused' };
    } catch {
      next = { failure: { kind: 'ipc' }, kind: 'refused' };
    }
    if (request === this.#request) {
      this.state = next;
    }
  }

  /** Opens the confirmed link in the system browser. */
  async open(): Promise<void> {
    const confirming = this.state;
    if (confirming.kind !== 'confirm') {
      return;
    }
    if (confirming.opening) {
      return;
    }
    const { link } = confirming;
    const request = this.#request;
    this.state = { failure: null, kind: 'confirm', link, opening: true };
    let failure: LinkFailure | null;
    try {
      const result = await commands.openLink({ url: link.url });
      failure = result.status === 'ok' ? null : { error: result.error, kind: 'core' };
    } catch {
      failure = { kind: 'ipc' };
    }
    if (request !== this.#request) {
      return;
    }
    if (failure === null) {
      this.close();
    } else {
      this.state = { failure, kind: 'confirm', link, opening: false };
    }
  }

  /** Closes the confirmation without opening anything. */
  close(): void {
    this.#request += 1;
    this.state = { kind: 'closed' };
  }
}

/** The window's link confirmation. */
export const linkConfirmation = new LinkConfirmation();
