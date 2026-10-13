// Calls into the validation harness of the app (apps/desktop/src/lib/
// validation/harness.ts), which only validation builds contain.
/** A Content Security Policy violation the webview reported. */
export interface CspViolation {
  readonly blockedUri: string;
  readonly directive: string;
  readonly sample: string;
  readonly source: string;
}

/** Which ways of running script the page allowed. */
export interface ScriptProbe {
  readonly evalRan: boolean;
  readonly functionConstructorRan: boolean;
  readonly inlineHandlerRan: boolean;
  readonly inlineScriptRan: boolean;
}

interface Distribution {
  readonly max: number;
  readonly p50: number;
  readonly p95: number;
}

/** What the webview received for a channel load. */
export interface ChannelLoadReport {
  readonly batches: number;
  readonly batchIntervals: Distribution;
  readonly deliveryLatency: Distribution;
  readonly deltas: number;
  readonly gaps: number;
  readonly outOfOrder: number;
  readonly sent: { readonly batches: number; readonly deltas: number } | null;
}

interface ChannelLoad {
  readonly deltasPerSecond: number;
  readonly durationMillis: number;
  readonly runs: number;
}

interface Harness {
  callUngrantedCommand(): Promise<string>;
  cspViolations(): readonly CspViolation[];
  insertHostileHtml(payloads: readonly string[]): Promise<boolean>;
  probeScripts(): Promise<ScriptProbe>;
  runChannelLoad(load: ChannelLoad): Promise<ChannelLoadReport>;
}

declare global {
  interface Window {
    mannieValidation?: Harness;
  }
}

/**
 * SvelteKit 3 fetches `_app/version.json` when the window gains focus, to
 * notice a new deployment; the policy's connect-src refuses it. The fetch
 * cannot succeed or matter in a desktop app, and the V1 record lists it.
 */
function isSvelteKitVersionCheck(violation: CspViolation): boolean {
  const isConnect = violation.directive === 'connect-src';
  const isVersionFile = violation.blockedUri.endsWith('/_app/version.json');
  return isConnect && isVersionFile;
}

// The functions given to browser.execute run in the webview, so each one
// looks the harness up itself.

/** The Content Security Policy violations so far, without SvelteKit's version checks. */
export async function cspViolations(): Promise<CspViolation[]> {
  const violations = await browser.execute(() => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.cspViolations();
  });
  return violations.filter((violation) => !isSvelteKitVersionCheck(violation));
}

/** Calls the command that no capability grants. Returns `allowed`, or Tauri's refusal. */
export async function callUngrantedCommand(): Promise<string> {
  return browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.callUngrantedCommand();
  });
}

/**
 * Tries eval, the Function constructor, an inline script and an inline event
 * handler from the app's own code, where the page's policy applies. Code that
 * WebDriver injects runs outside the policy, so it cannot show this.
 */
export async function probeScripts(): Promise<ScriptProbe> {
  return browser.execute(async () => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.probeScripts();
  });
}

/** Inserts each payload into the page from the app's own code. Returns whether any payload ran. */
export async function insertHostileHtml(payloads: readonly string[]): Promise<boolean> {
  return browser.execute(
    async (inserted) => {
      const harness = window.mannieValidation;
      if (harness === undefined) {
        throw new Error('the validation harness is missing: run the validation build');
      }
      return harness.insertHostileHtml(inserted);
    },
    [...payloads],
  );
}

/** Streams a synthetic load from the core and reports what arrived. */
export async function runChannelLoad(load: ChannelLoad): Promise<ChannelLoadReport> {
  return browser.execute(async (requested) => {
    const harness = window.mannieValidation;
    if (harness === undefined) {
      throw new Error('the validation harness is missing: run the validation build');
    }
    return harness.runChannelLoad(requested);
  }, load);
}
