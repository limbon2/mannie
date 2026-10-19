// @ts-check
// The direct native calls of plugins' library crates (ADR-0035, decision 6;
// ADR-0034, decision 13): file writes, processes and sockets. A plugin's
// library crates reach the system through the SDK's services, so Clippy runs
// on the library crates that its entry crate ships with a configuration of
// the core that refuses these calls. A plugin's manifest may grant itself a
// category under `[native]`, with the reason, as Workers grants the link its
// sockets; the review of the manifest is the gate, and `task plugins:grants`
// lists every grant.
//
// Each plugin's configuration is the root's `clippy.toml` with the calls of
// the categories that its manifest does not grant added to its disallowed
// methods and types. The lists hold the calls of std, tokio, tonic's connect
// and serve, and reqwest, the core's HTTP client. Sockets include Unix
// sockets and Windows named pipes.
//
// A grant is the only exception: the library crates' sources may not
// suppress the two lints, or a group that holds them, with `allow` or
// `expect`, so that `task plugins:grants` lists every exception. A build
// script runs only where the plugin builds, so it may.

import { parse, stringify } from 'smol-toml';

/**
 * The calls that a category refuses: methods and types, by their paths.
 * @typedef {{ methods: string[], types: string[] }} NativeCalls
 */

/** The categories of native calls, by their keys under `[native]`. */
export const NATIVE_CATEGORIES = /** @type {const} */ (['file-writes', 'processes', 'sockets']);

/**
 * @typedef {typeof NATIVE_CATEGORIES[number]} NativeCategory
 */

/** The calls that each category holds. */
/** @type {Record<NativeCategory, NativeCalls>} */
export const NATIVE_CALLS = {
  'file-writes': {
    methods: [
      'std::fs::copy',
      'std::fs::create_dir',
      'std::fs::create_dir_all',
      'std::fs::hard_link',
      'std::fs::remove_dir',
      'std::fs::remove_dir_all',
      'std::fs::remove_file',
      'std::fs::rename',
      'std::fs::set_permissions',
      'std::fs::soft_link',
      'std::fs::write',
      'std::fs::File::create',
      'std::fs::File::create_new',
      'std::fs::File::options',
      'std::os::unix::fs::chown',
      'std::os::unix::fs::fchown',
      'std::os::unix::fs::lchown',
      'std::os::unix::fs::symlink',
      'std::os::windows::fs::symlink_dir',
      'std::os::windows::fs::symlink_file',
      'tokio::fs::copy',
      'tokio::fs::create_dir',
      'tokio::fs::create_dir_all',
      'tokio::fs::hard_link',
      'tokio::fs::remove_dir',
      'tokio::fs::remove_dir_all',
      'tokio::fs::remove_file',
      'tokio::fs::rename',
      'tokio::fs::set_permissions',
      'tokio::fs::symlink',
      'tokio::fs::symlink_dir',
      'tokio::fs::symlink_file',
      'tokio::fs::write',
      'tokio::fs::File::create',
      'tokio::fs::File::create_new',
      'tokio::fs::File::options',
    ],
    types: ['std::fs::DirBuilder', 'std::fs::OpenOptions', 'tokio::fs::DirBuilder', 'tokio::fs::OpenOptions'],
  },
  processes: {
    methods: ['std::process::abort', 'std::process::exit'],
    types: ['std::process::Command', 'tokio::process::Command'],
  },
  sockets: {
    methods: [
      'reqwest::blocking::get',
      'reqwest::get',
      'std::net::ToSocketAddrs::to_socket_addrs',
      'tokio::net::lookup_host',
    ],
    types: [
      'reqwest::blocking::Client',
      'reqwest::blocking::ClientBuilder',
      'reqwest::Client',
      'reqwest::ClientBuilder',
      'std::net::TcpListener',
      'std::net::TcpStream',
      'std::net::UdpSocket',
      'std::os::unix::net::UnixDatagram',
      'std::os::unix::net::UnixListener',
      'std::os::unix::net::UnixStream',
      'tokio::net::TcpListener',
      'tokio::net::TcpSocket',
      'tokio::net::TcpStream',
      'tokio::net::UdpSocket',
      'tokio::net::UnixDatagram',
      'tokio::net::UnixListener',
      'tokio::net::UnixSocket',
      'tokio::net::UnixStream',
      'tokio::net::windows::named_pipe::ClientOptions',
      'tokio::net::windows::named_pipe::ServerOptions',
      'tonic::transport::Endpoint',
      'tonic::transport::Server',
    ],
  },
};

/**
 * Whether Clippy may skip a path that does not resolve. A path of std
 * outside `std::os` resolves in every crate on every target, so Clippy
 * reports a typo in it. Every other path may name a crate that a plugin's
 * crate does not depend on, or another platform's module.
 * @param {string} path
 */
function mayNotResolve(path) {
  const isPortableStd = path.startsWith('std::') && !path.startsWith('std::os::');
  return !isPortableStd;
}

/**
 * The categories that a manifest grants, with their reasons, in the order
 * of `NATIVE_CATEGORIES`.
 * @param {{ native?: Partial<Record<NativeCategory, string>> }} manifest
 * @returns {{ category: NativeCategory, reason: string }[]}
 */
export function nativeGrants(manifest) {
  const granted = manifest.native ?? {};
  return NATIVE_CATEGORIES.flatMap((category) => {
    const reason = granted[category];
    return reason === undefined ? [] : [{ category, reason }];
  });
}

/**
 * The Clippy configuration of a plugin's library crates: the root's, with
 * the calls of every category that the plugin's manifest does not grant.
 * @param {string} rootConfiguration the text of the root's `clippy.toml`
 * @param {string} id the plugin's ID
 * @param {NativeCategory[]} granted the categories that its manifest grants
 * @returns {string}
 */
export function pluginClippyConfiguration(rootConfiguration, id, granted) {
  const configuration = parse(rootConfiguration);
  const refused = NATIVE_CATEGORIES.filter((category) => !granted.includes(category));
  /**
   * @param {string} path
   * @param {NativeCategory} category
   */
  const entry = (path, category) => ({
    'allow-invalid': mayNotResolve(path),
    path,
    reason: `The manifest of ${id} does not grant ${category} under [native]: a plugin's library crates reach the system through the SDK's services (ADR-0035, decision 6).`,
  });
  const methods = refused.flatMap((category) => NATIVE_CALLS[category].methods.map((path) => entry(path, category)));
  const types = refused.flatMap((category) => NATIVE_CALLS[category].types.map((path) => entry(path, category)));
  const existingMethods = /** @type {unknown[]} */ (configuration['disallowed-methods'] ?? []);
  const existingTypes = /** @type {unknown[]} */ (configuration['disallowed-types'] ?? []);
  configuration['disallowed-methods'] = [...existingMethods, ...methods];
  configuration['disallowed-types'] = [...existingTypes, ...types];
  const header = [
    '# Generated by scripts/plugins/check-native.mjs from clippy.toml and the',
    `# manifest of the plugin ${id} (ADR-0035, decision 6). Do not edit or commit it.`,
    '',
  ].join('\n');
  return `${header}\n${stringify(configuration)}\n`;
}

/**
 * The lints whose suppression would hide a direct native call: the two lints
 * of the rule and the groups that hold them.
 */
export const NATIVE_LINTS = [
  'clippy::all',
  'clippy::disallowed_methods',
  'clippy::disallowed_types',
  'clippy::style',
  'warnings',
];

/** A char literal at a position, which a lifetime such as `'a` is not. */
const CHAR_LITERAL = /'(?:\\(?:[nrt0\\'"]|x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\})|[^\\'\n])'/uy;

/** A string literal at a position, up to its closing quote or the end of the text. */
const STRING_LITERAL = /"(?:[^"\\]|\\[\s\S])*(?:"|$)/uy;

/** The opening of a raw string at a position, with its hashes: `r"`, `r#"`, `br#"`. */
const RAW_STRING = /b?r(#*)"/uy;

/**
 * Where the pair that opens at `index`, such as `[` or a block comment,
 * closes, after the pairs that it nests: the index after its closing text,
 * or the end of the text.
 * @param {string} text
 * @param {number} index
 * @param {string} open the opening text
 * @param {string} close the closing text
 */
function closingEnd(text, index, open, close) {
  let depth = 0;
  let end = index;
  let isOpen = true;
  while (isOpen) {
    if (text.startsWith(open, end)) {
      depth += 1;
      end += open.length;
    } else if (text.startsWith(close, end)) {
      depth -= 1;
      end += close.length;
    } else {
      end += 1;
    }
    const isNested = depth > 0;
    const isInText = end < text.length;
    isOpen = isNested && isInText;
  }
  return Math.min(end, text.length);
}

/**
 * The opening of a raw string at `index`, or `null`. The `r` starts a
 * token, so the end of a name before a quote is not one.
 * @param {string} text
 * @param {number} index
 */
function rawStringAt(text, index) {
  const startsToken = !/\w/u.test(text.charAt(index - 1));
  if (!startsToken) {
    return null;
  }
  RAW_STRING.lastIndex = index;
  return RAW_STRING.exec(text);
}

/**
 * The comment or literal that starts at `index`: where it ends, and how many
 * of its first characters are its opening, such as the `r#"` of a raw
 * string, which stays. `null` when none starts there.
 * @param {string} text
 * @param {number} index
 * @returns {{ end: number, opening: number } | null}
 */
function commentOrLiteralAt(text, index) {
  const pair = text.slice(index, index + 2);
  if (pair === '//') {
    const end = text.indexOf('\n', index);
    return { end: end === -1 ? text.length : end, opening: 0 };
  }
  if (pair === '/*') {
    return { end: closingEnd(text, index, '/*', '*/'), opening: 0 };
  }
  const raw = rawStringAt(text, index);
  if (raw !== null) {
    const closing = `"${raw[1]}`;
    const close = text.indexOf(closing, index + raw[0].length);
    return { end: close === -1 ? text.length : close + closing.length, opening: raw[0].length };
  }
  for (const literal of [STRING_LITERAL, CHAR_LITERAL]) {
    literal.lastIndex = index;
    if (literal.test(text)) {
      return { end: literal.lastIndex, opening: 1 };
    }
  }
  return null;
}

/**
 * A Rust source's text with its comments and the contents of its string and
 * char literals replaced by spaces, so that only code is left. The text
 * keeps its length and its line breaks, so lines keep their numbers.
 * @param {string} text
 * @returns {string}
 */
export function codeOf(text) {
  /** @type {string[]} */
  const out = [];
  let index = 0;
  while (index < text.length) {
    const skipped = commentOrLiteralAt(text, index);
    if (skipped === null) {
      out.push(text.charAt(index));
      index += 1;
    } else {
      const opening = index + skipped.opening;
      out.push(text.slice(index, opening), text.slice(opening, skipped.end).replaceAll(/[^\n]/gu, ' '));
      index = skipped.end;
    }
  }
  return out.join('');
}

/**
 * The suppressions in a Rust source that would hide a direct native call:
 * the `allow` and `expect` attributes, also inside `cfg_attr`, that name a
 * lint of `NATIVE_LINTS`, each with its line.
 * @param {string} text
 * @returns {{ line: number, lint: string }[]}
 */
export function nativeSuppressions(text) {
  const code = codeOf(text);
  /** @type {{ line: number, lint: string }[]} */
  const found = [];
  for (const start of code.matchAll(/#!?\[/gu)) {
    const bracket = start.index + start[0].length - 1;
    const body = code.slice(start.index, closingEnd(code, bracket, '[', ']'));
    const line = code.slice(0, start.index).split('\n').length;
    for (const levels of body.matchAll(/\b(?:allow|expect)\s*\(([^()]*)\)/gu)) {
      const lints = levels[1].split(',').map((part) => part.replaceAll(/\s/gu, ''));
      for (const lint of lints.filter((name) => NATIVE_LINTS.includes(name))) {
        found.push({ line, lint });
      }
    }
  }
  return found;
}
