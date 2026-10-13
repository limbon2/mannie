// @ts-check
// V5 (ADR-0008): the worker's dependency graph contains no Tauri, GTK or
// WebKitGTK, on any target. It follows normal and build dependencies from
// mannie-worker through the resolve graph of `cargo metadata`, which covers
// every platform. deny.toml checks the other crate boundaries.
//
// `task lint:deps` pipes `cargo metadata --format-version 1 --locked` into
// this script.

import { text } from 'node:stream/consumers';
import { fileURLToPath } from 'node:url';

/** Crates of the desktop stack that must never reach the worker. */
export const FORBIDDEN_CRATE = /^(?:tauri|wry|tao|muda|gtk|gdk|gio|glib|webkit2gtk|javascriptcore-rs|soup3)(?:[-_]|$)/;

/**
 * @typedef {{ kind: string | null }} DependencyKind
 * @typedef {{ pkg: string, dep_kinds: DependencyKind[] }} ResolvedDependency
 * @typedef {{ id: string, deps: ResolvedDependency[] }} ResolvedNode
 * @typedef {{ id: string, name: string }} MetadataPackage
 * @typedef {{ packages: MetadataPackage[], resolve: { nodes: ResolvedNode[] } }} Metadata
 */

/**
 * The names of the crates that `root` depends on, directly or not, through
 * normal and build dependencies.
 * @param {Metadata} metadata
 * @param {string} root the package name
 * @returns {Set<string>}
 */
export function dependencyClosure(metadata, root) {
  const names = new Map(metadata.packages.map((cratePackage) => [cratePackage.id, cratePackage.name]));
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const rootPackage = metadata.packages.find((cratePackage) => cratePackage.name === root);
  if (rootPackage === undefined) {
    throw new Error(`cargo metadata has no package ${root}`);
  }
  /** @type {Set<string>} */
  const seen = new Set();
  const queue = [rootPackage.id];
  while (queue.length > 0) {
    const id = queue.pop() ?? '';
    for (const dependency of nodes.get(id)?.deps ?? []) {
      const shipped = dependency.dep_kinds.some((kind) => kind.kind !== 'dev');
      const isNew = !seen.has(dependency.pkg);
      if (shipped && isNew) {
        seen.add(dependency.pkg);
        queue.push(dependency.pkg);
      }
    }
  }
  return new Set([...seen].map((id) => names.get(id) ?? id));
}

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  /** @type {Metadata} */
  const metadata = JSON.parse(await text(process.stdin));
  const closure = dependencyClosure(metadata, 'mannie-worker');
  const forbidden = [...closure].filter((name) => FORBIDDEN_CRATE.test(name)).sort();
  for (const name of forbidden) {
    console.error(
      `error: mannie-worker depends on ${name}; the worker must not depend on the desktop stack (ADR-0008, V5)`,
    );
  }
  if (forbidden.length > 0) {
    process.exitCode = 1;
  } else {
    console.log(
      `check-worker-dependencies: none of the worker's ${String(closure.size)} dependencies belongs to Tauri, GTK or WebKitGTK`,
    );
  }
}
