// Types of the interface settings that the generated bindings do not cover.
import type { InterfaceSettingsError } from '#lib/generated/bindings.js';

/** Why a request about the interface settings failed: an error the core reported, or no answer at all. */
export type SettingsFailure =
  { readonly error: InterfaceSettingsError; readonly kind: 'core' } | { readonly kind: 'ipc' };
