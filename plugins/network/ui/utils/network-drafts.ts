// Pure draft constructors for collection and request forms.
import type { AuthConfig, BodyConfig, CollectionView, KeyValueEntry, RequestView } from '../generated/bindings.js';
import type { CollectionDraft, RequestDraft } from '../types/network.js';

function cloneEntries(entries: readonly KeyValueEntry[]): KeyValueEntry[] {
  return entries.map((entry) => ({ ...entry }));
}

function cloneBody(body: BodyConfig): BodyConfig {
  switch (body.kind) {
    case 'none': {
      return { kind: 'none' };
    }
    case 'json': {
      return { content: body.content, kind: 'json' };
    }
    case 'text': {
      return { content: body.content, contentType: body.contentType, kind: 'text' };
    }
    case 'form_url_encoded': {
      return { entries: cloneEntries(body.entries), kind: 'form_url_encoded' };
    }
  }
}

function cloneAuth(auth: AuthConfig): AuthConfig {
  return { ...auth };
}

/** Returns an empty collection draft for creating a new HTTP collection. */
export function emptyCollectionDraft(): CollectionDraft {
  return {
    auth: { kind: 'none' },
    autoCookies: true,
    baseUrl: '',
    description: '',
    enabled: true,
    headers: [],
    name: '',
    queryParams: [],
    variables: [],
  };
}

/** Clones an existing collection into an editable draft. */
export function draftFromCollection(col: CollectionView): CollectionDraft {
  return {
    auth: col.auth.kind === 'inherit' ? { kind: 'none' } : cloneAuth(col.auth),
    autoCookies: col.autoCookies,
    baseUrl: col.baseUrl,
    description: col.description,
    enabled: col.enabled,
    headers: cloneEntries(col.headers),
    name: col.name,
    queryParams: cloneEntries(col.queryParams),
    variables: col.variables.map((entry) => ({ ...entry })),
  };
}

/** Clones a saved request into an editable workbench draft. */
export function draftFromRequest(item: RequestView): RequestDraft {
  return {
    auth: cloneAuth(item.auth),
    body: cloneBody(item.body),
    description: item.description,
    folderId: item.folderId,
    headers: cloneEntries(item.headers),
    method: item.method,
    name: item.name,
    pathParams: cloneEntries(item.pathParams),
    queryParams: cloneEntries(item.queryParams),
    timeoutSeconds: item.timeoutSeconds,
    urlTemplate: item.urlTemplate,
    useCookies: item.useCookies,
  };
}
