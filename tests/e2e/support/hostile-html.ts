// A corpus of hostile HTML (V1, security architecture T-3): markup that tries
// to run script, to load something from the network or to navigate away.
// Every payload that runs sets `window.compromised`; every payload that
// reaches the network contacts `origin`, which the test points at a local
// server that counts connections.
//
// M0 inserts the corpus straight into the DOM, which tests the Content
// Security Policy and the navigation guard on their own. M1 runs the same
// corpus through the Markdown renderer of model output, which must also drop
// the raw HTML.

/** Payloads that try to run script or to load a resource from `origin`. */
export function hostileHtml(origin: string): string[] {
  return [
    '<script>window.compromised = true;</script>',
    `<script src="${origin}/payload.js"></script>`,
    '<img src="missing.png" onerror="window.compromised = true">',
    `<img src="${origin}/tracker.png" alt="">`,
    `<img srcset="${origin}/tracker.png 1x" alt="">`,
    '<svg onload="window.compromised = true"><circle r="1"></circle></svg>',
    '<svg><script>window.compromised = true;</script></svg>',
    `<svg><image href="${origin}/tracker.svg"></image></svg>`,
    '<iframe src="javascript:parent.compromised = true"></iframe>',
    '<iframe srcdoc="<script>parent.compromised = true;</script>"></iframe>',
    `<object data="${origin}/payload.swf"></object>`,
    `<embed src="${origin}/payload.swf">`,
    '<a id="hostile-link" href="javascript:window.compromised = true">link</a>',
    '<details open ontoggle="window.compromised = true"><summary>details</summary></details>',
    '<input autofocus onfocus="window.compromised = true">',
    `<video src="${origin}/clip.mp4" autoplay muted></video>`,
    `<audio src="${origin}/sound.mp3" autoplay></audio>`,
    `<input type="image" src="${origin}/button.png" alt="">`,
    `<link rel="stylesheet" href="${origin}/style.css">`,
    `<link rel="preload" as="image" href="${origin}/preload.png">`,
    `<style>@import url("${origin}/style.css");</style>`,
    `<style>body { background-image: url("${origin}/background.png"); }</style>`,
    `<div style="background-image: url(${origin}/background.png)">styled</div>`,
    `<style>@font-face { font-family: hostile; src: url("${origin}/font.woff2"); } body { font-family: hostile; }</style>`,
    `<base href="${origin}/">`,
    `<form action="${origin}/collect"><input name="field" value="data"></form>`,
    '<math><mtext><table><mglyph><style><img src="missing.png" onerror="window.compromised = true">',
    '<noscript><p title="</noscript><img src=missing.png onerror=window.compromised=true>"></noscript>',
    '<div onclick="window.compromised = true" id="hostile-click">click</div>',
  ];
}

/**
 * Payloads that try to navigate a frame or the window to `origin`. The policy
 * and the navigation guard refuse each navigation, but the browser engine
 * opens connections to the target before it does (V1 record). Only the
 * Markdown renderer, which never produces this markup, keeps these payloads
 * off the network.
 */
export function hostileNavigations(origin: string): string[] {
  return [
    `<iframe src="${origin}/frame"></iframe>`,
    `<meta http-equiv="refresh" content="0; url=${origin}/refresh">`,
    `<a id="hostile-link" href="${origin}/link">link</a>`,
  ];
}

/**
 * Markdown that tries to smuggle a URL or markup past the renderer of model
 * output (ADR-0021): links and images to `origin`, script in links, raw HTML
 * inline and in tables, task lists and footnotes, and encoded markup.
 */
export function hostileMarkdown(origin: string): string[] {
  return [
    '[click](javascript:window.compromised=true)',
    `[remote](${origin}/link "title")`,
    `![tracker](${origin}/tracker.png)`,
    `![tracker][image]\n\n[image]: ${origin}/reference.png`,
    `[reference][target]\n\n[target]: ${origin}/reference`,
    `<${origin}/autolink>`,
    `www.example.com/page and ${origin}/bare`,
    '<a href="javascript:window.compromised=true">inline html</a>',
    '<img src=x onerror=window.compromised=true>',
    '`<script>window.compromised = true;</script>`',
    '```html\n<script>window.compromised = true;</script>\n```',
    `[![nested](${origin}/image.png)](${origin}/)`,
    `| a | b |\n| - | - |\n| <script>window.compromised = true;</script> | [l](${origin}/cell) |`,
    `- [x] done <style>body { background: url("${origin}/style.png"); }</style>\n- [ ] open`,
    'Text[^note]\n\n[^note]: A footnote with <b onmouseover="window.compromised = true">html</b>',
    '<details open ontoggle="window.compromised = true">',
    '&lt;script&gt;window.compromised = true;&lt;/script&gt; &#x3C;img src=x onerror=window.compromised=true&#x3E;',
  ];
}
