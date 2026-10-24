// The files that the owner drops on a notebook's page (plugins/notebooks/docs/
// README.md, sources), and their bytes as the plugin's command takes them.

/** The extensions of the files that the plugin reads; the core checks the bytes again. */
const EXTENSIONS = ['.markdown', '.md', '.pdf', '.txt'];

/** Whether the plugin reads a file of this name, by its extension. */
export function takesDocument(name: string): boolean {
  const lower = name.toLowerCase();
  return EXTENSIONS.some((extension) => lower.endsWith(extension));
}

/** Whether a drag carries files, which the page then takes. */
export function carriesFiles(transfer: DataTransfer | null): boolean {
  return transfer?.types.includes('Files') ?? false;
}

/** The bytes of a binary string's characters at a time, below the limit of a call's arguments. */
const CHUNK = 0x80_00;

/** A file's bytes as base64. */
export async function readAsBase64(file: Blob): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const parts: string[] = [];
  for (let start = 0; start < bytes.length; start += CHUNK) {
    parts.push(String.fromCodePoint(...bytes.subarray(start, start + CHUNK)));
  }
  return btoa(parts.join(''));
}
