#!/usr/bin/env node
// Renders public/og.png, the 1200×630 card that link previews show for every page (og:image in
// partials/head.html), from the design tokens, the self-hosted Gabarito and the logo partial.
// Rerun after changing the tagline, the logo or the colors:
//   node scripts/og-image.mjs
// Uses the playwright package from the repo root; CHROMIUM_PATH picks a browser.
import { readFileSync } from 'node:fs';
import { chromium } from 'playwright';

const site = new URL('../', import.meta.url);
const tokens = new URL('../../packages/tokens/', site);
const font = readFileSync(new URL('node_modules/@fontsource-variable/gabarito/files/gabarito-latin-wght-normal.woff2', tokens)).toString('base64');
const css = readFileSync(new URL('generated/tokens.css', tokens), 'utf8');
const logo = readFileSync(new URL('partials/logo.html', site), 'utf8');

// The hero's network, small: people as dots, one coral post on its way.
const nodes = [[90, 120, 'mi'], [250, 70, 'jw'], [200, 250, 'sk'], [360, 200, 'you'], [320, 370, 'pt'], [130, 360, 'ra']];
const links = [[0, 1], [0, 2], [1, 3], [2, 3], [2, 4], [3, 4], [0, 5], [5, 2], [5, 4]];
const net = `<svg class="net" width="440" height="440" viewBox="0 0 440 440" aria-hidden="true">
  ${links.map(([a, b]) => `<line x1="${nodes[a][0]}" y1="${nodes[a][1]}" x2="${nodes[b][0]}" y2="${nodes[b][1]}"/>`).join('')}
  ${nodes.map(([x, y, n]) => `<g class="${n === 'you' ? 'me' : ''}"><circle cx="${x}" cy="${y}" r="${n === 'you' ? 38 : 30}"/><text x="${x}" y="${y + 8}">${n}</text></g>`).join('')}
  <circle class="post" cx="305" cy="135" r="14"/>
</svg>`;

const html = `<!doctype html><html><head><meta charset="utf-8"><style>
${css}
@font-face { font-family: 'Gabarito'; font-weight: 400 900; src: url(data:font/woff2;base64,${font}) format('woff2-variations') }
* { box-sizing: border-box }
html, body { margin: 0; width: 1200px; height: 630px; overflow: hidden }
body { background: var(--bg); color: var(--ink); font-family: var(--display); padding: 64px 80px; display: flex; flex-direction: column; position: relative }
.logo { display: flex; align-items: center; gap: 14px; font-size: 46px; font-weight: 900; letter-spacing: -0.04em }
.logo svg { width: 64px; height: 64px }
.logo span { color: var(--mute) }
.eyebrow { align-self: flex-start; margin-top: 40px; padding: 8px 20px; border: 3px solid var(--ink); border-radius: 999px; background: var(--sun); font-size: 24px; font-weight: 800; transform: rotate(-2deg); transform-origin: left center }
h1 { margin: 26px 0 0; font-size: 84px; font-weight: 900; line-height: 0.93; letter-spacing: -0.045em }
h1 b { color: var(--cobalt); font-weight: 900 }
.net { position: absolute; right: 64px; top: 120px }
.net line { stroke: var(--ink); stroke-width: 4; stroke-linecap: round; opacity: .18 }
.net circle { fill: var(--card); stroke: var(--ink); stroke-width: 4 }
.net .me circle { fill: var(--sun) }
.net .post { fill: var(--coral); stroke: none }
.net text { font: 800 22px var(--display); text-anchor: middle; fill: var(--ink) }
</style></head><body>
<div class="logo">${logo}<b>hoi<span>.sh</span></b></div>
<div class="eyebrow">a forum where communities hold their own keys</div>
<h1>Say hoi. Find<br>your people.<br><b>Your community<br>holds the keys.</b></h1>
${net}
</body></html>`;

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, colorScheme: 'light' });
  await page.setContent(html);
  await page.evaluate(() => document.fonts.ready);
  const out = new URL('public/og.png', site);
  await page.screenshot({ path: out.pathname, type: 'png' });
  console.log(`wrote ${out.pathname}`);
} finally {
  await browser.close();
}
