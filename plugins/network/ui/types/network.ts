// Types of the Network Client plugin's UI (ADR-0075; plugins/network/docs/README.md).
import type { PluginCallFailure, PluginOutcome } from '@mannie/plugin-sdk/ipc';

import type {
  AuthConfig,
  BodyConfig,
  CollectionView,
  FolderView,
  HttpMethod,
  KeyValueEntry,
  NetworkError,
  RequestView,
  VariableEntry,
} from '../generated/bindings.js';

/** A saved request paired with its owning collection. */
export interface CollectionRequestPair {
  readonly collection: CollectionView;
  readonly request: RequestView;
}

/** Why a call of the plugin failed: the plugin's error, the host's refusal or no answer. */
export type NetworkFailure = PluginCallFailure<NetworkError>;

/** Outcome of a plugin IPC command call. */
export type NetworkOutcome<T> = PluginOutcome<T, NetworkError>;

/** Discriminator of an auth configuration. */
export type AuthKind = AuthConfig['kind'];

/** Discriminator of a request body configuration. */
export type BodyKind = BodyConfig['kind'];

/** All HTTP methods supported in version 1. */
export const HTTP_METHODS: readonly HttpMethod[] = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];

/** Editable form state for a collection dialog. */
export interface CollectionDraft {
  auth: AuthConfig;
  autoCookies: boolean;
  baseUrl: string;
  description: string;
  enabled: boolean;
  headers: KeyValueEntry[];
  name: string;
  queryParams: KeyValueEntry[];
  variables: VariableEntry[];
}

/** Editable form state for a saved request in the workbench. */
export interface RequestDraft {
  auth: AuthConfig;
  body: BodyConfig;
  description: string;
  folderId: string | null;
  headers: KeyValueEntry[];
  method: HttpMethod;
  name: string;
  pathParams: KeyValueEntry[];
  queryParams: KeyValueEntry[];
  timeoutSeconds: number;
  urlTemplate: string;
  useCookies: boolean;
}

/** Target of the collection create or edit modal. */
export type CollectionDialogState =
  { readonly collection: CollectionView; readonly mode: 'edit' } | { readonly mode: 'create' };

/** Target of the folder create or rename modal. */
export type FolderDialogState =
  { readonly collectionId: string; readonly mode: 'create' } | { readonly folder: FolderView; readonly mode: 'rename' };

/** Target of a destructive confirmation dialog. */
export type DeleteDialogState =
  | { readonly id: string; readonly kind: 'collection'; readonly name: string }
  | { readonly id: string; readonly kind: 'environment'; readonly name: string }
  | { readonly id: string; readonly kind: 'folder'; readonly name: string }
  | { readonly id: string; readonly kind: 'request'; readonly name: string }
  | { readonly kind: 'clear_cookies' };

/** Parsed non-secret card details for `network_send_request`. */
export interface SendCardDetails {
  readonly bytes: number | null;
  readonly collectionName: string | null;
  readonly durationMs: number | null;
  readonly method: string | null;
  readonly protocol: string | null;
  readonly requestName: string | null;
  readonly status: number | null;
  readonly statusText: string | null;
  readonly truncated: boolean;
  readonly url: string | null;
}
