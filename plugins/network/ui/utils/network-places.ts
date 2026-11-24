// Where the Network Client plugin shows collections and saved requests, using
// the shell's view navigation parameters (ADR-0045; ADR-0075).
import { type NavigateOptions, openView, viewParameters } from '@mannie/plugin-sdk/navigation';

/** The plugin's view and activity bar ID. */
export const NETWORK_VIEW = 'network';

/** View search parameter naming the active collection. */
export const COLLECTION_PARAMETER = 'collection';

/** View search parameter naming the active saved request. */
export const REQUEST_PARAMETER = 'request';

/** The collection currently named by the view's address, or `null`. */
export function shownCollectionId(): string | null {
  return viewParameters().get(COLLECTION_PARAMETER);
}

/** The saved request currently named by the view's address, or `null`. */
export function shownRequestId(): string | null {
  return viewParameters().get(REQUEST_PARAMETER);
}

/** Opens the bare Network Client view. */
export function openNetwork(options?: NavigateOptions): Promise<void> {
  return openView(NETWORK_VIEW, undefined, options);
}

/** Opens a collection overview in the Network Client view. */
export function openCollection(collectionId: string, options?: NavigateOptions): Promise<void> {
  return openView(NETWORK_VIEW, { [COLLECTION_PARAMETER]: collectionId }, options);
}

/** Opens one saved request in the Postman-style request workbench. */
export function openRequest(collectionId: string, requestId: string, options?: NavigateOptions): Promise<void> {
  return openView(
    NETWORK_VIEW,
    {
      [COLLECTION_PARAMETER]: collectionId,
      [REQUEST_PARAMETER]: requestId,
    },
    options,
  );
}
