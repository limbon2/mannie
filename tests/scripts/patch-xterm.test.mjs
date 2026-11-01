// @ts-check
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

import { patchedWebgl, patchedXterm } from '../../scripts/patch-xterm.mjs';

/** VS Code's `KeyCodeUtils` in the builds, as their minifier writes it. */
const KEY_CODE_UTILS =
  '(o=>{function s(l){return Jn.keyCodeToStr(l)}o.toString=s;function t(l){return 1}o.fromString=t})(Qn||={})';

/** The style elements of xterm.js 6.0.0's build, as its minifier writes them. */
const STYLE_ELEMENTS = [
  'this._styleElement=n.mainDocument.createElement("style"),i.appendChild(this._styleElement),this._register(x)',
  'this._dimensionsStyleElement||(this._dimensionsStyleElement=this._document.createElement("style"),this._screenElement.appendChild(this._dimensionsStyleElement));let i=1',
  'this._themeStyleElement||(this._themeStyleElement=this._document.createElement("style"),this._screenElement.appendChild(this._themeStyleElement));let i=2',
];

/**
 * The installed build of a package of the desktop app.
 * @param {string} path
 * @returns {string}
 */
function installed(path) {
  return readFileSync(new URL(`../../apps/desktop/node_modules/${path}`, import.meta.url), 'utf8');
}

describe('patchedXterm', () => {
  it('replaces the style elements with stylesheets, defines toString and adds the function of the stylesheets', () => {
    const text = patchedXterm([...STYLE_ELEMENTS, KEY_CODE_UTILS].join(';'));

    assert.ok(text.startsWith('/*'), 'the function comes first');
    assert.match(text, /function mannieStyleSheet\(document\)/u);
    assert.match(text, /this\._styleElement=mannieStyleSheet\(n\.mainDocument\),this\._register\(x\)/u);
    assert.match(text, /this\._dimensionsStyleElement=mannieStyleSheet\(this\._document\)\);let i=1/u);
    assert.match(text, /this\._themeStyleElement=mannieStyleSheet\(this\._document\)\);let i=2/u);
    assert.match(text, /Object\.defineProperty\(o,"toString",\{[^}]*value:s,writable:!0\}\);function t/u);
    assert.doesNotMatch(text, /createElement\("style"\)/u);
    assert.doesNotMatch(text, /\.toString=/u);
  });

  it('refuses a build where a place is missing', () => {
    const build = [STYLE_ELEMENTS[0], STYLE_ELEMENTS[1], KEY_CODE_UTILS].join(';');

    assert.throws(() => patchedXterm(build), /theme: expected one place, found 0/u);
  });

  it('refuses a build that makes another style element', () => {
    const build = [...STYLE_ELEMENTS, KEY_CODE_UTILS, 'document.createElement("style")'].join(';');

    assert.throws(() => patchedXterm(build), /another style element/u);
  });
});

describe('patchedWebgl', () => {
  it("defines KeyCodeUtils's toString, which a frozen Object.prototype lets an object define but not assign", () => {
    const text = patchedWebgl(`var a=1;${KEY_CODE_UTILS};`);

    assert.match(text, /Object\.defineProperty\(o,"toString"/u);
    assert.doesNotMatch(text, /\.toString=/u);
  });

  it('refuses a build without KeyCodeUtils', () => {
    assert.throws(() => patchedWebgl('var a=1;'), /expected one place, found 0/u);
  });
});

describe('the installed builds', () => {
  it('carry both patches once pnpm installed them', () => {
    const xterm = installed('@xterm/xterm/lib/xterm.mjs');
    const webgl = installed('@xterm/addon-webgl/lib/addon-webgl.mjs');

    assert.match(xterm, /^\/\*\n \* Mannie's patch/u);
    assert.doesNotMatch(xterm, /createElement\("style"\)/u);
    assert.doesNotMatch(xterm, /\.toString=/u);
    assert.doesNotMatch(webgl, /\.toString=/u);
  });
});
