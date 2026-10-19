// The files of a paste or a drop that the Images plugin takes
// (plugins/images/docs/README.md, adding images), and their bytes as the core's
// command takes them.

const MEDIA_TYPES = new Set(['image/gif', 'image/jpeg', 'image/png', 'image/webp']);

const EXTENSIONS = ['.gif', '.jpeg', '.jpg', '.png', '.webp'];

/**
 * Whether the plugin takes a file: by its media type, or by its name's
 * extension when the system gave no type. The core checks the bytes again.
 */
export function takesImageFile(file: File): boolean {
  if (file.type !== '') {
    return MEDIA_TYPES.has(file.type);
  }
  const name = file.name.toLowerCase();
  return EXTENSIONS.some((extension) => name.endsWith(extension));
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
