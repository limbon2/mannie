// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { explainMetadataFailure, lockTask } from '../../../scripts/plugins/lib/cargo.mjs';
import { parseLock } from '../../../scripts/plugins/lib/lock.mjs';

const REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index';

/** The committed locks: the root's pins tokio, and Images' pins image. */
const COMMITTED = [
  {
    lock: parseLock(`version = 4\n\n[[package]]\nname = "tokio"\nversion = "1.53.2"\nsource = "${REGISTRY}"\n`),
    name: 'Cargo.lock',
  },
  {
    lock: parseLock(`version = 4\n\n[[package]]\nname = "image"\nversion = "0.25.10"\nsource = "${REGISTRY}"\n`),
    name: 'plugins/images/Cargo.lock',
  },
];

/** @type {import('../../../scripts/plugins/lib/cargo.mjs').Workspace} */
const IMAGES = { folder: 'plugins/images', kind: 'plugin', plugin: 'images' };
/** @type {import('../../../scripts/plugins/lib/cargo.mjs').Workspace} */
const ROOT = { folder: '.', kind: 'root' };

/**
 * The first line of the message for a workspace and cargo's error.
 * @param {import('../../../scripts/plugins/lib/cargo.mjs').Workspace} workspace
 * @param {string} stderr
 */
function messageOf(workspace, stderr) {
  return explainMetadataFailure(workspace, stderr, COMMITTED).split('\n')[0];
}

// The four forms of cargo's offline failures that spike S8 recorded.
describe('explainMetadataFailure', () => {
  it("names the plugin's lock task for a lock that its manifest outgrew", () => {
    const stderr = String.raw`error: cannot update the lock file D:\mannie\plugins\images\Cargo.lock because --locked was passed to prevent this`;

    assert.equal(
      messageOf(IMAGES, stderr),
      'plugins/images/Cargo.lock does not record every crate and dependency that plugins/images/Cargo.toml names. Run `task plugins:lock ID=images`, then `task gen`.',
    );
  });

  it("names the root's lock step for the root's lock", () => {
    const stderr = String.raw`error: cannot update the lock file D:\mannie\Cargo.lock because --locked was passed to prevent this`;

    assert.match(messageOf(ROOT, stderr), /^Cargo\.lock does not record .* Run `cargo update --workspace --offline`/u);
  });

  it('names task setup for a pinned crate that the cache lacks', () => {
    const stderr =
      'error: failed to download `image v0.25.10`\n\nCaused by:\n  attempting to make an HTTP request, but --offline was specified';

    assert.equal(
      messageOf(IMAGES, stderr),
      "Cargo's cache lacks image 0.25.10, which a committed lock pins. Run `task setup`, which downloads it, then `task gen`.",
    );
  });

  it('names task setup for a pinned crate whose index entry the cache lacks', () => {
    const stderr = String.raw`error: no matching package named ${'`image`'} found
location searched: crates.io index
required by package ${'`mannie-images v0.2.0 (D:\\mannie\\plugins\\images\\crates\\mannie-images)`'}`;

    assert.equal(
      messageOf(IMAGES, stderr),
      "Cargo's cache does not know image, which the committed locks pin (0.25.10 in plugins/images/Cargo.lock). Run `task setup`, which downloads it, then `task gen`.",
    );
  });

  it('names the lock task of the plugin that asks for a crate that no lock pins', () => {
    const stderr = String.raw`error: no matching package named ${'`planted`'} found
location searched: crates.io index
required by package ${'`mannie-images v0.2.0 (D:\\mannie\\plugins\\images\\crates\\mannie-images)`'}`;

    assert.equal(
      messageOf(ROOT, stderr),
      "A crate of the plugin images depends on planted, which no committed lock pins and Cargo's cache does not hold. Run `task plugins:lock ID=images` with the network, then `task gen`.",
    );
  });

  it('names the lock task for a requirement that no pinned version meets', () => {
    const stderr = String.raw`error: failed to select a version for the requirement ${'`tokio = "^1.99"`'}
candidate versions found which didn't match: 1.53.2
required by package ${'`mannie-images v0.2.0 (D:\\mannie\\plugins\\images\\crates\\mannie-images)`'}`;

    assert.equal(
      messageOf(IMAGES, stderr),
      'A crate of the plugin images asks for tokio ^1.99, and the committed locks pin 1.53.2 in Cargo.lock. Run `task plugins:lock ID=images`, then `task gen`.',
    );
  });

  it('shows any other error as cargo wrote it', () => {
    assert.equal(
      explainMetadataFailure(IMAGES, 'error: something else', COMMITTED),
      'cargo metadata failed for plugins/images/Cargo.toml:\nerror: something else',
    );
  });
});

describe('lockTask', () => {
  it("names a plugin's task and the root's command", () => {
    assert.equal(lockTask('workers'), 'task plugins:lock ID=workers');
    assert.equal(lockTask(undefined), 'cargo update --workspace --offline');
  });
});
