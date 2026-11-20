import type { LayerRect } from '@mannie/plugin-sdk/surface';

import { formatGeometryValue, type GeometryEdit, type GeometryField, geometryInput } from '../utils/canvas-geometry.js';

interface FieldDraft {
  invalid: boolean;
  text: string;
}

/** The accepted values include completed edits waiting their turn to reach the store. */
export class GeometryFields {
  #accepted = $state.raw<LayerRect | null>(null);
  #drafts = $state<Partial<Record<GeometryField, FieldDraft | null>>>({});
  #active: GeometryField | null = null;

  reset(accepted: LayerRect | null): void {
    this.#accepted = accepted;
    this.cancelDrafts();
  }

  accept(geometry: LayerRect | null): void {
    this.#accepted = geometry;
  }

  text(field: GeometryField): string {
    const value = this.#accepted?.[field];
    return this.#drafts[field]?.text ?? (value === undefined ? '' : formatGeometryValue(value));
  }

  invalid(field: GeometryField): boolean {
    return this.#drafts[field]?.invalid === true;
  }

  change(field: GeometryField, text: string): void {
    if (!this.#accepted) {
      return;
    }
    const invalid = this.invalid(field) && geometryInput(this.#accepted, field, text) === null;
    this.#drafts[field] = { invalid, text };
    this.#active = field;
  }

  get preview(): LayerRect | null {
    const field = this.#active;
    if (!field || !this.#accepted) {
      return null;
    }
    const draft = this.#drafts[field];
    return draft ? geometryInput(this.#accepted, field, draft.text) : null;
  }

  commit(field: GeometryField): GeometryEdit | null {
    const draft = this.#drafts[field];
    if (!draft || !this.#accepted) {
      return null;
    }
    const next = geometryInput(this.#accepted, field, draft.text);
    if (!next) {
      draft.invalid = true;
      return null;
    }
    this.cancel(field);
    if (next[field] === this.#accepted[field]) {
      return null;
    }
    this.#accepted = next;
    return { field, value: next[field] };
  }

  cancel(field: GeometryField): void {
    this.#drafts[field] = null;
    if (this.#active === field) {
      this.#active = null;
    }
  }

  cancelDrafts(keepErrors = false): void {
    this.#active = null;
    if (keepErrors) {
      // Showing a field error can resize the viewport. Keep its text available for correction.
      this.#drafts = Object.fromEntries(Object.entries(this.#drafts).filter(([, draft]) => draft?.invalid));
    } else {
      this.#drafts = {};
    }
  }
}
