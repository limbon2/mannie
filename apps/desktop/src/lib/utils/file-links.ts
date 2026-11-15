// Presentation-only detection. Rust alone decides whether a path can open.
const SCHEME = /^[a-z][a-z\d+.-]*:/iu;
const DRIVE = /^[a-z]:[/\\]/iu;
const LOCATION = /(?::[1-9]\d*(?::[1-9]\d*)?|#L[1-9]\d*)$/u;
const CONTROLS = /\p{Cc}/u;

/** Whether an explicit Markdown destination belongs to local navigation. */
export function isFileLink(value: string): boolean {
  const fileUrl = /^file:/iu.test(value);
  const drive = DRIVE.test(value);
  if (fileUrl || drive) {
    return true;
  }
  const path = value.replace(LOCATION, '');
  const hasPath = path !== '';
  const fragment = path.startsWith('#');
  const scheme = SCHEME.test(path);
  const originalScheme = SCHEME.test(value);
  const filenameLocation = path.includes('.');
  const ambiguousScheme = originalScheme && !filenameLocation;
  return hasPath && !fragment && !scheme && !ambiguousScheme;
}

/** Only whole inline-code spans with an unambiguous path shape are linked. */
export function isInlineFilePath(value: string): boolean {
  const empty = value === '';
  const tooLong = value.length > 8192;
  const controls = CONTROLS.test(value);
  const padded = value.trim() !== value;
  if (empty || tooLong || controls || padded) {
    return false;
  }
  if (!isFileLink(value)) {
    return false;
  }
  const path = value.replace(LOCATION, '');
  const hasOption = /\s-/u.test(path);
  // Operators, glob patterns and quotes indicate code or a command. Paths
  // containing these characters can still use an explicit Markdown link.
  const hasOperator = /[\x22'\x60|<>*?=;(){}]/u.test(path);
  if (hasOption || hasOperator) {
    return false;
  }
  const drive = DRIVE.test(path);
  const rooted = path.startsWith('/');
  const explicitlyRelative = ['./', '../', '.\\', '..\\'].some((prefix) => path.startsWith(prefix));
  // Bare URL prefixes are often syntax examples. Automatic links need a
  // local path after the prefix; explicit Markdown links still reach Rust.
  const fileUrl = /^file:\/\/(?:localhost)?\/[^/\\?#]/iu.test(path);
  if (drive || rooted || explicitlyRelative || fileUrl) {
    return true;
  }
  const spaced = /\s/u.test(path);
  const option = path.startsWith('-');
  const colon = path.includes(':');
  if (spaced || option || colon) {
    return false;
  }
  const separated = path.includes('/') || path.includes('\\');
  const dotfile = /^\.[\p{L}\p{N}_-][\p{L}\p{N}_.-]*$/u.test(path);
  const filename = /^[\p{L}\p{N}_-][\p{L}\p{N}_.-]*\.[\p{L}\p{N}_-]+$/u.test(path);
  return separated || dotfile || filename;
}
