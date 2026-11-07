// Accessibility audit: axe-core on every route of the app, in the light and
// the dark theme, with reduced motion, against the in-memory mock core so every screen has
// something on it. Fails on serious and critical findings; writes everything it found to
// test-results/a11y/report.json. `--shots` also saves a screenshot of each route.
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { consentAccepted } from './browser-vault.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);
const axePath = require.resolve('axe-core/axe.min.js');
const out = join(root, 'test-results/a11y');
const shots = process.argv.includes('--shots');
await mkdir(out, { recursive: true });

process.env.VITE_HOI_BACKEND = 'mock';
process.env.VITE_HOI_REGISTRY_URL = '';
const server = await createServer({ root: join(root, 'apps/web'), logLevel: 'error', server: { host: '127.0.0.1', port: 0, hmr: false } });
await server.listen();
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });

/** Waits in the page until the app says yes (an async predicate would read as true at once). */
async function until(page, what, check) {
  const ok = await page.evaluate(async (src) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const test = new Function('app', `return (${src})`);
    for (let i = 0; i < 300; i++) {
      if (test(app)) return true;
      await new Promise((r) => setTimeout(r, 100));
    }
    return false;
  }, check);
  assert.ok(ok, what);
}

/** The routes, with ids from the mock core's own data. */
async function routes(page) {
  await until(page, 'the mock core lists communities', 'app.joined.length > 0');
  const ids = await page.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const joined = app.joined;
    const c = joined[0]?.id;
    const feed = c ? await app.core.feed({ community: c, sort: 'hot', limit: 5 }) : [];
    const post = feed.find((p) => p.link) ?? feed[0];
    const other = feed.find((p) => !p.mine)?.author.key;
    return { c, post: post?.id, person: other, me: app.me?.rootKey };
  });
  assert.ok(ids.c && ids.post && ids.person, `the mock core has data: ${JSON.stringify(ids)}`);
  return [
    ['home', '#/'],
    ['home, fresh', '#/?sort=fresh'],
    ['communities', '#/communities'],
    ['registry', '#/registry'],
    ['community', `#/c/${ids.c}`],
    ['community ladder', `#/c/${ids.c}/ladder`],
    ['community manage', `#/c/${ids.c}/manage`],
    ['ledger', `#/c/${ids.c}/ledger`],
    ['wiki', `#/c/${ids.c}/wiki`],
    ['wiki page', `#/c/${ids.c}/wiki/getting-started`],
    ['post', `#/p/${ids.post}`],
    ['person', `#/u/${ids.person}`],
    ['person in a community', `#/u/${ids.person}?c=${ids.c}`],
    ['you', '#/you'],
    ['new community', '#/new'],
    ['new private community', '#/new/private'],
    ['private communities', '#/private'],
    ['join', '#/join'],
    ['public join', '#/public'],
    ['pair', '#/pair'],
    ['node link', '#/node'],
    ['companion', '#/companion'],
    ['server', '#/server'],
    ['privacy', '#/privacy'],
    ['saved', '#/saved'],
    ['search', '#/search?q=audit'],
    ['not found', '#/no-such-place'],
  ];
}

async function audit(page) {
  return page.evaluate(async () => {
    // eslint-disable-next-line no-undef
    const r = await axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'] }, resultTypes: ['violations'] });
    return r.violations.map((v) => ({ id: v.id, impact: v.impact, help: v.help, nodes: v.nodes.map((n) => ({ target: n.target.join(' '), html: n.html.slice(0, 200), summary: n.failureSummary })) }));
  });
}

/** Touch targets below 44 px on a phone, which axe doesn't check at that size. */
async function smallTargets(page) {
  return page.evaluate(() => {
    const small = [];
    for (const el of document.querySelectorAll('a[href], button, input, select, textarea, [role="switch"], summary')) {
      const r = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      if (!r.width || !r.height || style.visibility === 'hidden' || el.closest('[aria-hidden="true"], .sr-only')) continue;
      // a link or button inside a sentence is exempt (WCAG 2.5.8 inline exception), and so is a
      // checkbox inside its label, which is the target
      if ((el.tagName === 'A' || el.tagName === 'BUTTON') && style.display.startsWith('inline') && el.parentElement?.closest('p, li, td, .help')) continue;
      if (el.tagName === 'INPUT' && ['checkbox', 'radio'].includes(el.type) && el.closest('label')) continue;
      if (r.height < 24 || r.width < 24) small.push(`${el.tagName.toLowerCase()} "${(el.textContent || el.getAttribute('aria-label') || '').trim().slice(0, 40)}" ${Math.round(r.width)}×${Math.round(r.height)}`);
    }
    return small;
  });
}

const findings = [];
let failed = 0;
try {
  for (const scheme of ['light', 'dark']) {
    for (const [viewport, size] of [['desktop', { width: 1280, height: 900 }], ['phone', { width: 390, height: 844 }]]) {
      const context = await browser.newContext({ colorScheme: scheme, reducedMotion: 'reduce', viewport: size, bypassCSP: true });
      await consentAccepted(context);
      await context.addInitScript({ path: axePath });
      const page = await context.newPage();
      const errors = [];
      page.on('pageerror', (e) => errors.push(e.message));
      await page.goto(`${origin}/?me=a11y-audit#/`);
      await until(page, 'the app is ready', "app.phase === 'ready'");
      for (const [name, hash] of await routes(page)) {
        await page.evaluate((h) => { location.hash = h; }, hash);
        await page.waitForTimeout(400);
        await page.waitForLoadState('networkidle');
        const violations = await audit(page);
        const targets = viewport === 'phone' ? await smallTargets(page) : [];
        for (const v of violations) {
          findings.push({ route: name, scheme, viewport, ...v });
          if (v.impact === 'serious' || v.impact === 'critical') failed++;
        }
        if (targets.length) findings.push({ route: name, scheme, viewport, id: 'target-size-24', impact: 'moderate', help: 'Touch targets at least 24 px (44 px is the design rule)', nodes: targets.map((t) => ({ target: t })) });
        if (shots) await page.screenshot({ path: join(out, `${name.replace(/[^a-z]+/g, '-')}-${scheme}-${viewport}.png`), fullPage: true });
      }
      assert.deepEqual(errors, [], `no page errors (${scheme}, ${viewport})`);
      await context.close();
    }
  }
} finally {
  await writeFile(join(out, 'report.json'), JSON.stringify(findings, null, 2));
  await browser.close();
  await server.close();
}

const byRule = new Map();
for (const f of findings) {
  const k = `${f.impact} · ${f.id}: ${f.help}`;
  byRule.set(k, (byRule.get(k) ?? new Set()).add(`${f.route} (${f.scheme}, ${f.viewport})`));
}
for (const [rule, where] of [...byRule].sort()) console.log(`${rule}\n    ${[...where].slice(0, 6).join('; ')}${where.size > 6 ? ` and ${where.size - 6} more` : ''}`);
console.log(`${findings.length} findings, ${failed} serious or critical; report: test-results/a11y/report.json`);
if (failed) process.exit(1);
console.log('PASS: no serious or critical accessibility findings on any route, in either theme, on desktop or phone.');
