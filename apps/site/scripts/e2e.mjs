#!/usr/bin/env node
// Browser checks for the built site: run `pnpm --filter @hoi/site build` first, then
//   node scripts/e2e.mjs [--shots <dir>] [--quick] [--shots-only] [--playground] [--download]
// Serves dist/ with `vite preview` and loads every page at 1280×900 and 390×844, light and dark,
// scripts on and off, with and without reduced motion (plus a 320×640 layout pass). Fails on console errors, failed requests,
// any CSP violation, request to another origin, or horizontal overflow. Runs the home demos and the download page
// with an empty and a filled release manifest. Uses the playwright package from the repo root.
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { SCENARIOS } from '../src/playground/scenarios.ts';
import { YOU_AT } from '../src/playground/world.ts';
import { signer } from '../../../scripts/fixtures/minisign-test-signer.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const siteDir = resolve(here, '..');
const args = process.argv.slice(2);
const shotsDir = resolve(args.includes('--shots') ? args[args.indexOf('--shots') + 1] : join(siteDir, 'test-results', 'shots'));
const quick = args.includes('--quick');
const shotsOnly = args.includes('--shots-only');
const PORT = Number(process.env.PORT || 4317);
const ORIGIN = `http://localhost:${PORT}`;

// The wiki's pages share one layout: a few of them go through the full matrix, and every one of
// them through a quicker pass (wikiPass, below).
const WIKI_SAMPLE = ['/wiki/', '/wiki/architecture/', '/wiki/playground/', '/wiki/sbom/', '/wiki/joining/', '/wiki/records/'];
const PAGES = ['/', '/download/', '/verify/', '/relay/', '/communities/', '/protocol/', '/privacy/', '/404.html', ...WIKI_SAMPLE];
// The built site reads its release list from publication.json's `downloads` (hoi.sh). The tests
// stay offline: every browser context gets the site's own placeholder there, as hoi.sh serves it
// before the first release is published.
const PUBLISHED = JSON.parse(readFileSync(join(siteDir, '../../publication.json'), 'utf8')).downloads.replace(/\/+$/, '');
const PUBLISHED_ORIGIN = PUBLISHED ? new URL(PUBLISHED).origin : '';
const PLACEHOLDER = readFileSync(join(siteDir, 'public/releases/manifest.json'));
const WIKI_ALL = JSON.parse(readFileSync(join(siteDir, 'wiki/nav.json'), 'utf8')).groups.flatMap((g) => g.pages.map((p) => (p.slug ? `/wiki/${p.slug}/` : '/wiki/')));
const VIEWPORTS = { desktop: { width: 1280, height: 900 }, mobile: { width: 390, height: 844 }, small: { width: 320, height: 640 } };

const failures = [];
const fail = (where, what) => { failures.push(`${where}: ${what}`); console.log(`  ✗ ${where}: ${what}`); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/* ---------- preview server ---------- */
const server = spawn(process.execPath, [join(siteDir, 'node_modules/vite/bin/vite.js'), 'preview', '--port', String(PORT), '--strictPort', '--host', 'localhost'], {
  cwd: siteDir,
  stdio: ['ignore', 'pipe', 'pipe'],
});
let serverLog = '';
server.stdout.on('data', (d) => { serverLog += d; });
server.stderr.on('data', (d) => { serverLog += d; });
async function waitForServer() {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(ORIGIN + '/')).ok) return; } catch { /* not yet */ }
    await sleep(100);
  }
  throw new Error('vite preview did not start:\n' + serverLog);
}

/* ---------- helpers ---------- */
const watchers = new WeakMap();
async function watch(page, where, allowed = []) {
  const previous = watchers.get(page);
  if (previous) { Object.assign(previous, { where, allowed }); return; }
  const current = { where, allowed };
  watchers.set(page, current);
  const report = (message) => fail(current.where, message);
  await page.exposeFunction('__reportCspViolation', (violation) => report(`CSP violation: ${JSON.stringify(violation)}`));
  await page.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', (event) => {
      void window.__reportCspViolation({
        directive: event.effectiveDirective, blocked: event.blockedURI,
        source: event.sourceFile, line: event.lineNumber, column: event.columnNumber,
      });
    });
  });
  page.on('console', (m) => { if (m.type() === 'error') report(`console error: ${m.text()}`); });
  page.on('pageerror', (e) => report(`page error: ${e.message}`));
  page.on('requestfailed', (r) => report(`request failed: ${r.url()} (${r.failure()?.errorText})`));
  page.on('request', (r) => {
    const u = new URL(r.url());
    if (u.protocol === 'data:' || u.protocol === 'blob:') return;
    if (u.origin !== ORIGIN && u.origin !== PUBLISHED_ORIGIN && !current.allowed.includes(u.origin)) report(`third-party request: ${r.url()}`);
  });
  page.on('response', (r) => { if (r.status() >= 400) report(`HTTP ${r.status()} ${r.url()}`); });
}

// Drive scrolling from here: with scripts disabled the page's own timers never fire.
async function scrollThrough(page) {
  const { height, step } = await page.evaluate(() => ({
    height: document.documentElement.scrollHeight,
    step: Math.max(200, Math.floor(innerHeight * 0.7)),
  }));
  for (let y = 0; y < height; y += step) {
    await page.evaluate((top) => window.scrollTo(0, top), y);
    await sleep(30);
  }
  await page.evaluate(() => window.scrollTo(0, 0));
}

async function checkOverflow(page, where) {
  const o = await page.evaluate(() => ({
    doc: document.documentElement.scrollWidth,
    body: document.body.scrollWidth,
    vw: window.innerWidth,
  }));
  if (o.doc > o.vw || o.body > o.vw) fail(where, `horizontal overflow: scrollWidth ${o.doc}/${o.body} > ${o.vw}`);
}

// Touch targets on phones: 44px for every control that isn't a link inside running text.
async function checkTargets(page, where) {
  const small = await page.evaluate(() => {
    const out = [];
    for (const el of document.querySelectorAll('a, button, [tabindex="0"]')) {
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height) continue;
      const cs = getComputedStyle(el);
      if (cs.visibility === 'hidden' || cs.display === 'none') continue;
      if (el.tagName === 'A' && cs.display === 'inline' && el.closest('p, li, td, dd, figcaption, .kv-h, .lead')) continue;
      const pad = el.classList.contains('sw') ? 18 : 0; // the switch has a larger pseudo-element hit area
      if (r.height + pad < 44) out.push(`${el.tagName.toLowerCase()} "${(el.textContent || '').trim().slice(0, 24)}" ${Math.round(r.height)}px`);
    }
    return out;
  });
  if (small.length) fail(where, `touch targets under 44px: ${small.join(', ')}`);
}

/* ---------- the matrix ---------- */
async function matrix(browser) {
  const combos = [];
  for (const [vpName, viewport] of Object.entries(VIEWPORTS))
    for (const colorScheme of ['light', 'dark'])
      for (const js of [true, false])
        for (const reducedMotion of ['no-preference', 'reduce'])
          combos.push({ vpName, viewport, colorScheme, js, reducedMotion });
  // 320px only needs the layout checks: light, motion, with and without scripts.
  const full = combos.filter((c) => c.vpName !== 'small' || (c.colorScheme === 'light' && c.reducedMotion === 'no-preference'));
  const list = quick ? full.filter((c) => c.colorScheme === 'light' && c.reducedMotion === 'no-preference') : full;
  for (const c of list) {
    const label = `${c.vpName}/${c.colorScheme}/${c.js ? 'js' : 'nojs'}/${c.reducedMotion === 'reduce' ? 'reduced' : 'motion'}`;
    console.log(`• ${label}`);
    const ctx = await browser.newContext({ viewport: c.viewport, colorScheme: c.colorScheme, javaScriptEnabled: c.js, reducedMotion: c.reducedMotion });
    for (const path of PAGES) {
      const page = await ctx.newPage();
      const where = `${label} ${path}`;
      await watch(page, where);
      await page.goto(ORIGIN + path, { waitUntil: 'load' });
      await page.evaluate(() => document.fonts.ready);
      await scrollThrough(page);
      await sleep(c.js ? 250 : 50);
      await checkOverflow(page, where);
      if (c.vpName !== 'desktop') await checkTargets(page, where);
      const h1 = await page.locator('h1').first().innerText().catch(() => '');
      if (!h1.trim()) fail(where, 'no visible h1');
      await page.close();
    }
    await ctx.close();
  }
}

/* ---------- every wiki page, its search and the playground ---------- */
async function wikiPass(browser) {
  for (const [vpName, viewport] of [['desktop', VIEWPORTS.desktop], ['mobile', VIEWPORTS.mobile]]) {
    console.log(`• wiki ${vpName}/dark/js/motion (${WIKI_ALL.length} pages)`);
    const ctx = await browser.newContext({ viewport, colorScheme: 'dark' });
    for (const path of WIKI_ALL) {
      const page = await ctx.newPage();
      const where = `wiki ${vpName} ${path}`;
      await watch(page, where);
      await page.goto(ORIGIN + path, { waitUntil: 'load' });
      await scrollThrough(page);
      await sleep(150);
      await checkOverflow(page, where);
      if (vpName !== 'desktop') await checkTargets(page, where);
      const h1 = await page.locator('h1').first().innerText().catch(() => '');
      if (!h1.trim()) fail(where, 'no visible h1');
      await page.close();
    }
    await ctx.close();
  }
  console.log('• wiki search and playground');
  const ctx = await browser.newContext({ viewport: VIEWPORTS.desktop });
  const page = await ctx.newPage();
  await watch(page, 'wiki search');
  await page.goto(ORIGIN + '/wiki/');
  await page.keyboard.press('/');
  await page.keyboard.type('reconciliation');
  await sleep(300);
  const hits = await page.locator('.wk-dlg-res a').count();
  if (!hits) fail('wiki search', 'no results for "reconciliation"');
  await page.keyboard.press('Escape');
  await watch(page, 'playground');
  await page.goto(ORIGIN + '/wiki/playground/');
  await page.locator('[data-pg-speed]').fill('3');
  await page.locator('[data-pg-play]').click();
  await sleep(9000);
  const done = await page.locator('.pg-steps li.done').count();
  if (done < 3) fail('playground', `only ${done} steps played in 9 s`);
  await page.locator('[data-pg-reset]').click();
  await page.getByRole('button', { name: 'Phone', exact: true }).click();
  await page.locator('[data-pg-step]').click();
  await sleep(2500);
  if (!(await page.locator('.pg-insp pre').count())) fail('playground', 'stepping shows no frame');
  await ctx.close();
}

/* ---------- full-screen playground, including native touch gestures ---------- */
async function playgroundPass(browser) {
  const frames = (page) => page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const camera = (page) => page.locator('#pg-canvas').evaluate((c) => {
    const { a, e, f } = c.getContext('2d').getTransform();
    const dpr = c.width / c.getBoundingClientRect().width;
    return { s: a / dpr, x: e / dpr, y: f / dpr };
  });
  for (const viewport of [VIEWPORTS.desktop, { width: 1024, height: 768 }, VIEWPORTS.mobile, VIEWPORTS.small, { width: 844, height: 390 }]) {
    const where = `playground ${viewport.width}×${viewport.height}`;
    console.log(`• ${where}`);
    const ctx = await browser.newContext({ viewport, hasTouch: true, reducedMotion: viewport.height < 500 ? 'no-preference' : 'reduce' });
    const page = await ctx.newPage();
    await watch(page, where);
    await page.goto(ORIGIN + '/wiki/playground/');
    const shell = page.locator('#pg');
    assert.deepEqual(await shell.boundingBox(), { x: 0, y: 0, ...viewport }, `${where}: fills the viewport`);
    assert.equal(await page.locator('.wk-side').isVisible(), false);
    assert.equal(await page.locator('.site-header').isVisible(), false);
    assert.equal(await page.locator('.preview-bar').isVisible(), false);
    for (const panel of ['setup', 'details', 'activity', 'guide', ...(viewport.width <= 900 ? ['network'] : [])]) {
      await page.locator(`[data-pg-panel="${panel}"]`).click();
      assert.equal(await page.locator(`#pg-${panel}`).isVisible(), true, `${where}: ${panel} is reachable`);
      assert.equal(await page.locator(`[data-pg-panel="${panel}"]`).getAttribute('aria-pressed'), 'true');
      const fits = await page.locator(`#pg-${panel}`).evaluate((el) => el.scrollWidth <= el.clientWidth);
      assert.ok(fits, `${where}: ${panel} has no horizontal overflow`);
      const controls = await page.locator('.pg-toolbar, .pg-tabs').evaluateAll((els) => els.every((el) => {
        const r = el.getBoundingClientRect();
        return r.top >= 0 && r.bottom <= innerHeight;
      }));
      assert.ok(controls, `${where}: playback and navigation stay in view`);
      await checkTargets(page, `${where}/${panel}`);
    }
    await page.locator('[data-pg-panel="setup"]').click();
    await page.getByRole('button', { name: 'Phone', exact: true }).click();
    await page.locator('.pg-steps li').last().click();
    assert.equal(await page.locator('#pg-details').isVisible(), true, `${where}: the last step can be inspected`);
    assert.ok(await page.locator('#pg-details pre').count());
    await checkTargets(page, `${where}/frame`);
    await page.locator('[data-pg-panel="setup"]').click();
    assert.equal(await page.getByRole('button', { name: 'Phone', exact: true }).getAttribute('aria-pressed'), 'true');
    await page.getByRole('button', { name: 'Browser', exact: true }).click();
    await page.locator('[data-pg-step]').click();
    await page.locator('.pg-steps li.now').waitFor({ state: 'attached' });
    assert.equal(await page.locator('#pg-network').isVisible(), true, `${where}: Step shows the network`);
    await page.locator('[data-pg-current]').click();
    assert.equal(await page.locator('#pg-details').isVisible(), true);
    await page.locator('[data-pg-reset]').click();
    if (viewport.width <= 900) await page.locator('[data-pg-panel="network"]').click();
    else await page.locator('[data-pg-panel="setup"]').click();
    await frames(page);
    await page.screenshot({ path: join(shotsDir, `playground-${viewport.width}.png`) });

    if (viewport.width === 390) {
      const canvas = page.locator('#pg-canvas');
      const box = await canvas.boundingBox();
      const initial = await camera(page);
      const cdp = await ctx.newCDPSession(page);
      const point = (id, x, y) => ({ id, x: box.x + x, y: box.y + y });
      const touch = (type, touchPoints) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints });
      await touch('touchStart', [point(1, 130, 230)]);
      await touch('touchMove', [point(1, 175, 270)]);
      await touch('touchEnd', []);
      await frames(page);
      const panned = await camera(page);
      assert.ok(panned.x > initial.x + 30 && panned.y > initial.y + 25, 'touch pans in both directions');
      await touch('touchStart', [point(1, 100, 250), point(2, 220, 250)]);
      await touch('touchMove', [point(1, 70, 250), point(2, 250, 250)]);
      await touch('touchEnd', []);
      await frames(page);
      assert.ok((await camera(page)).s > panned.s * 1.3, 'pinching zooms the network');
      assert.equal(await page.locator('#pg-network').isVisible(), true, 'gestures do not inspect an actor');
      await touch('touchStart', [point(1, 100, 250)]);
      await touch('touchCancel', []);
      assert.equal(await canvas.evaluate((c) => c.classList.contains('drag')), false);
      await page.locator('[data-pg-fit]').click();
      await frames(page);
      const fitted = await camera(page);
      assert.ok(Math.abs(fitted.s - initial.s) < 0.001, 'Fit restores the overview');
      const [x, y] = YOU_AT[SCENARIOS[0].you];
      await page.touchscreen.tap(box.x + fitted.x + x * fitted.s, box.y + fitted.y + y * fitted.s);
      assert.equal(await page.locator('#pg-details').isVisible(), true, 'a touch tap inspects without mouse hover');
      assert.match(await page.locator('#pg-details h3').innerText(), /you/i);
      await page.setViewportSize({ width: 1200, height: 800 });
      await page.locator('#pg-network').waitFor({ state: 'visible' });
      assert.equal(await page.locator('#pg-network').isVisible(), true, 'network returns when the viewport widens');
      await page.setViewportSize(viewport);
      await page.locator('[data-pg-panel="network"]').click();
      await canvas.focus();
      await frames(page);
      const beforeKey = await camera(page);
      await page.keyboard.press('ArrowRight');
      await frames(page);
      assert.ok((await camera(page)).x < beforeKey.x - 30, 'keyboard pans the network');
      await page.goto(ORIGIN + '/wiki/playground/#reading');
      assert.equal(await page.locator('#pg-guide').isVisible(), true, 'guide deep links open the guide');
      await page.getByRole('link', { name: 'Back to the wiki' }).click();
      assert.ok(await page.locator('.site-header').isVisible(), 'returning restores wiki navigation');
    }
    await ctx.close();
  }
}

/* ---------- home demos ---------- */
async function demos(browser) {
  console.log('• home demos');
  const ctx = await browser.newContext({ viewport: VIEWPORTS.desktop });
  const page = await ctx.newPage();
  const where = 'demos /';
  await watch(page, where);
  await page.goto(ORIGIN + '/');
  await sleep(1200);
  // live network: nodes get built and posts hop
  const nodes = await page.locator('#net .node').count();
  if (nodes < 9) fail(where, `network has ${nodes} nodes`);
  await page.waitForFunction(() => /^[1-9]/.test(document.querySelector('#netstat')?.textContent ?? ''), null, { timeout: 8000 })
    .catch(() => fail(where, 'no post delivered in the live network'));
  // sign & send
  if (await page.locator('#resetBtn').isVisible()) fail(where, '"Again" is showing before anything was signed');
  await page.locator('#signBtn').scrollIntoViewIfNeeded();
  await page.getByRole('button', { name: 'Sign & send' }).click();
  await page.waitForFunction(() => !document.querySelector('#resetBtn')?.hidden, null, { timeout: 6000 })
    .catch(() => fail(where, 'sign & send never finished'));
  const sign = await page.evaluate(() => ({
    stamp: Number(getComputedStyle(document.querySelector('#stamp')).opacity),
    lit: document.querySelectorAll('#note .peer.lit').length,
    hash: document.querySelector('#hash')?.textContent ?? '',
    work: document.querySelector('#workbar').getBoundingClientRect().width / document.querySelector('.work').getBoundingClientRect().width,
  }));
  if (sign.stamp < 0.99 || sign.lit !== 3 || !/^b3:[0-9a-f]{4}…[0-9a-f]{4}$/.test(sign.hash) || sign.work < 0.99) fail(where, `sign demo end state wrong: ${JSON.stringify(sign)}`);
  await page.screenshot({ path: join(shotsDir, 'demo-signed.png'), clip: await page.locator('#signed').boundingBox() });
  await page.getByRole('button', { name: 'Again' }).click();
  await sleep(400);
  if (await page.locator('#signBtn').isDisabled()) fail(where, 'Again did not reset the sign demo');
  // flood it
  await page.locator('#floodBtn').scrollIntoViewIfNeeded();
  const before = Number(await page.locator('#count').innerText());
  await page.getByRole('button', { name: 'Flood it' }).click();
  await sleep(700);
  await page.screenshot({ path: join(shotsDir, 'demo-flood-mid.png'), clip: await page.locator('#spam').boundingBox() });
  await page.waitForFunction(() => Number(getComputedStyle(document.querySelector('#badge')).opacity) > 0.99, null, { timeout: 6000 })
    .catch(() => fail(where, 'flood badge never appeared'));
  const after = Number(await page.locator('#count').innerText());
  if (after !== before + 12) fail(where, `bounced count ${before} → ${after}, expected +12`);
  if (await page.locator('#arena .spam').count()) fail(where, 'spam chips left behind');
  // fade scrubber: scroll the posts past and the first one fades
  const fadeTop = await page.evaluate(() => document.querySelector('#posts').getBoundingClientRect().top + scrollY);
  await page.evaluate((y) => scrollTo(0, y - innerHeight * 0.2), fadeTop);
  await sleep(600);
  await page.evaluate((y) => scrollTo(0, y + 600), fadeTop);
  await sleep(900);
  const fade = await page.evaluate(() => {
    const p = document.querySelectorAll('#posts .fp');
    return { first: Number(getComputedStyle(p[0]).opacity), life: p[0].querySelector('.life i').getBoundingClientRect().width };
  });
  if (!(fade.first < 0.9)) fail(where, `fade section did not scrub: ${JSON.stringify(fade)}`);
  await page.screenshot({ path: join(shotsDir, 'demo-fade-scrolled.png') });
  // draggable chips: drag one and let go
  const chip = page.locator('#zone .chip').first();
  await chip.scrollIntoViewIfNeeded();
  await sleep(900);
  const box = await chip.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 120, box.y + box.height / 2 + 60, { steps: 8 });
  const moved = await chip.boundingBox();
  await page.mouse.up();
  if (Math.abs(moved.x - box.x) < 40) fail(where, `chip did not follow the drag (${box.x} → ${moved.x})`);
  await sleep(1200);
  await ctx.close();

  // relay switch: off → notice first → "Turn it on" → on; clicking again turns it off
  {
    const rp = await browser.newPage({ viewport: VIEWPORTS.desktop });
    await watch(rp, 'demos /relay/');
    await rp.goto(ORIGIN + '/relay/');
    const sw = rp.getByRole('switch', { name: 'Keep communities reachable' });
    await sw.click();
    await sleep(300);
    if ((await sw.getAttribute('aria-checked')) !== 'false') fail('demos /relay/', 'switch turned on without the notice');
    if (!(await rp.evaluate(() => document.activeElement?.id === 'noticeOn'))) fail('demos /relay/', 'notice did not take focus');
    await rp.getByRole('button', { name: 'Turn it on' }).click();
    await sleep(900);
    if ((await sw.getAttribute('aria-checked')) !== 'true') fail('demos /relay/', 'switch not on after "Turn it on"');
    if (!/17 friends/.test(await rp.locator('#relayText').innerText())) fail('demos /relay/', 'relay status not updated');
    await sw.click();
    if ((await sw.getAttribute('aria-checked')) !== 'false') fail('demos /relay/', 'switch did not turn off');
    await rp.close();
  }

  // reduced motion: same buttons, instant end states
  const rctx = await browser.newContext({ viewport: VIEWPORTS.mobile, reducedMotion: 'reduce' });
  const rp = await rctx.newPage();
  await watch(rp, 'demos / reduced');
  await rp.goto(ORIGIN + '/');
  await rp.getByRole('button', { name: 'Sign & send' }).click();
  await sleep(100);
  const rs = await rp.evaluate(() => Number(getComputedStyle(document.querySelector('#stamp')).opacity));
  if (rs < 0.99) fail('demos / reduced', 'sign demo not instant under reduced motion');
  await rp.getByRole('button', { name: 'Flood it' }).click();
  await sleep(100);
  if (Number(await rp.locator('#count').innerText()) !== 35) fail('demos / reduced', 'flood not instant under reduced motion');
  await rctx.close();
}

/* ---------- download page ---------- */
const DOWNLOADS = 'https://downloads.invalid/releases';
const DOWNLOADS_ORIGIN = new URL(DOWNLOADS).origin;

/** A copy of the site built to trust a throwaway release key, served by a second preview server. */
async function keyedSite() {
  const key = signer();
  const work = mkdtempSync(join(tmpdir(), 'hoi-site-keyed-'));
  const publication = { ...JSON.parse(readFileSync(join(siteDir, '../../publication.json'), 'utf8')), downloads: DOWNLOADS, manifestKey: key.key };
  writeFileSync(join(work, 'publication.json'), JSON.stringify(publication));
  const outDir = join(work, 'dist');
  const vite = join(siteDir, 'node_modules/vite/bin/vite.js');
  const env = { ...process.env, HOI_PUBLICATION_FILE: join(work, 'publication.json') };
  delete env.HOI_PUBLICATION;
  const built = await new Promise((done) => {
    const child = spawn(process.execPath, [vite, 'build', '--outDir', outDir, '--emptyOutDir', '--logLevel', 'error'], { cwd: siteDir, env, stdio: ['ignore', 'inherit', 'inherit'] });
    child.on('exit', done);
  });
  if (built !== 0) throw new Error('the keyed site build failed');
  const port = PORT + 1;
  const preview = spawn(process.execPath, [vite, 'preview', '--outDir', outDir, '--port', String(port), '--strictPort', '--host', 'localhost'], { cwd: siteDir, env, stdio: 'ignore' });
  const origin = `http://localhost:${port}`;
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(origin + '/download/')).ok) break; } catch { /* not yet */ }
    await sleep(100);
  }
  return { origin, sign: key.sign, close: () => { preview.kill(); rmSync(work, { recursive: true, force: true }); } };
}

const SAMPLE = {
  schema: 1,
  latest: '0.1.0',
  signature: 'manifest.json.minisig',
  releases: [{
    version: '0.1.0',
    date: '2026-11-01',
    notes: 'https://github.com/OWNER/hoi/releases/tag/v0.1.0',
    files: [
      ['macos', 'universal', 'dmg', 'hoi_0.1.0_universal.dmg', 14_221_003],
      ['windows', 'x86_64', 'msi', 'hoi_0.1.0_x64_en-US.msi', 9_900_112],
      ['windows', 'x86_64', 'exe', 'hoi_0.1.0_x64-setup.exe', 8_120_333],
      ['linux', 'x86_64', 'AppImage', 'hoi_0.1.0_amd64.AppImage', 88_123_456],
      ['linux', 'x86_64', 'deb', 'hoi_0.1.0_amd64.deb', 10_500_000],
      ['linux', 'aarch64', 'rpm', 'hoi-0.1.0-1.aarch64.rpm', 10_400_000],
      ['node', 'x86_64', 'tar.gz', 'hoi-node_0.1.0_x86_64-linux.tar.gz', 6_000_000],
    ].map(([platform, arch, kind, name, size], i) => ({
      platform, arch, kind, name, size,
      url: `https://github.com/OWNER/hoi/releases/download/v0.1.0/${name}`,
      sha256: (i.toString(16) + 'f3a9c0de').repeat(8).slice(0, 64),
    })),
  }],
};

async function download(browser) {
  console.log('• download page');
  // 1. the site's placeholder list (no release published yet) → honest empty state
  {
    const ctx = await browser.newContext({ viewport: VIEWPORTS.mobile, colorScheme: 'dark' });
    const page = await ctx.newPage();
    await watch(page, 'download empty');
    await page.goto(ORIGIN + '/download/');
    await page.getByText('No signed release yet').first().waitFor({ timeout: 4000 }).catch(() => fail('download empty', 'no empty state'));
    if (!(await page.getByText('pnpm --filter @hoi/desktop tauri build').first().isVisible())) fail('download empty', 'build commands not shown');
    if (!(await page.locator('#install-wait').isVisible())) fail('download empty', 'the installer does not say it needs the first signed release');
    const releases = await page.locator('#release').getByRole('link', { name: 'Releases on GitHub' }).getAttribute('href').catch(() => null);
    if (!releases?.endsWith('/releases')) fail('download empty', `no link to the releases page (${releases})`);
    await page.screenshot({ path: join(shotsDir, 'download-empty-mobile-dark.png'), fullPage: true });
    await ctx.close();
  }
  await downloadCopy(browser);
  // 2. a signed release list → files, sizes, hashes, copy buttons; OS highlighted. The page trusts only
  // the key built into it, so this case builds a copy of the site with a throwaway key
  // (HOI_PUBLICATION_FILE) and serves a list signed with that key through a route.
  const keyed = await keyedSite();
  try {
    const ctx = await browser.newContext({ viewport: VIEWPORTS.desktop, permissions: ['clipboard-read', 'clipboard-write'], userAgent: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36' });
    await ctx.addInitScript(() => Object.defineProperty(navigator, 'userAgentData', { value: { platform: 'Linux' } }));
    const page = await ctx.newPage();
    await watch(page, 'download filled', [keyed.origin, DOWNLOADS_ORIGIN]);
    const bytes = Buffer.from(JSON.stringify({ ...SAMPLE, release: JSON.parse(readFileSync(join(siteDir, '../../release.json'), 'utf8')) }));
    await page.route(`${DOWNLOADS}/manifest.json`, (r) => r.fulfill({ body: bytes, contentType: 'application/json' }));
    await page.route(`${DOWNLOADS}/manifest.json.minisig`, (r) => r.fulfill({ body: keyed.sign(bytes), contentType: 'text/plain' }));
    await page.goto(keyed.origin + '/download/#windows');
    await page.locator('#windows .file').first().waitFor({ timeout: 4000 }).catch(() => fail('download filled', 'no file rows'));
    const files = await page.locator('.file').count();
    if (files !== SAMPLE.releases[0].files.length) fail('download filled', `${files} file rows, expected ${SAMPLE.releases[0].files.length}`);
    if (!(await page.locator('#linux.is-you').count())) fail('download filled', 'visitor OS (Linux) not highlighted');
    if (!(await page.locator('#release').getByRole('link', { name: 'All releases on GitHub' }).count())) fail('download filled', 'no link to every release on GitHub');
    if (!(await page.locator('#release').getByText('signature checked').count())) fail('download filled', 'does not say the signature was checked');
    const sha = SAMPLE.releases[0].files[1].sha256;
    await page.locator('#windows .file').first().getByRole('button', { name: /Copy SHA-256/ }).click();
    const clip = await page.evaluate(() => navigator.clipboard.readText());
    if (clip !== sha) fail('download filled', `copied ${clip}, expected ${sha}`);
    await page.screenshot({ path: join(shotsDir, 'download-filled-desktop.png'), fullPage: true });
    await ctx.close();
    // The same build refuses a list signed with another key.
    const other = await browser.newContext({ viewport: VIEWPORTS.desktop });
    const tampered = await other.newPage();
    await tampered.route(`${DOWNLOADS}/manifest.json`, (r) => r.fulfill({ body: bytes, contentType: 'application/json' }));
    await tampered.route(`${DOWNLOADS}/manifest.json.minisig`, (r) => r.fulfill({ body: signer().sign(bytes), contentType: 'text/plain' }));
    await tampered.goto(keyed.origin + '/download/');
    await tampered.getByText('The release list could not be verified.').first().waitFor({ timeout: 4000 }).catch(() => fail('download filled', 'a list signed with another key was shown'));
    if (await tampered.locator('.file').count()) fail('download filled', 'file rows from a list signed with another key');
    await other.close();
  } finally {
    keyed.close();
  }
}

// Every copy button on the page copies exactly what it shows and looks the same when it did, in
// every box and in dark mode. A copy only counts when the browser copied: an extension that
// intercepts copying and answers as if it had (uBlock Origin's ClickFix filter does this for
// `irm … | iex`) must leave the button saying the text is selected, not "Copied".
const COPIES = [
  ['Copy the macOS and Linux install command', /^curl -fsSL https?:\/\/\S+\/install\.sh \| bash$/],
  ['Copy the Windows install command', /^irm https?:\/\/\S+\/install\.ps1 \| iex$/],
  ['Copy the build commands', /^git clone \S+\.git\ncd hoi\npnpm install && pnpm build$/],
  ['Copy the desktop build command', /^pnpm --filter @hoi\/desktop tauri build$/],
];
async function copyOn(page, name) {
  await page.evaluate(() => navigator.clipboard.writeText('unchanged'));
  const btn = page.getByRole('button', { name, exact: true });
  await btn.click();
  await page.waitForFunction((b) => b.textContent.trim() !== 'Copy', await btn.elementHandle(), { timeout: 1000 }).catch(() => {});
  return {
    btn,
    label: (await btn.textContent())?.trim(),
    clip: await page.evaluate(() => navigator.clipboard.readText()),
    selected: await page.evaluate(() => getSelection()?.toString() ?? ''),
    look: await btn.evaluate((b) => { const s = getComputedStyle(b); return `${s.backgroundColor} ${s.color}`; }),
  };
}
async function downloadCopy(browser) {
  const perms = { permissions: ['clipboard-read', 'clipboard-write'] };
  for (const colorScheme of ['light', 'dark']) {
    const where = `download copy ${colorScheme}`;
    const ctx = await browser.newContext({ viewport: VIEWPORTS.desktop, colorScheme, ...perms });
    const page = await ctx.newPage();
    await watch(page, where);
    await page.goto(ORIGIN + '/download/');
    await page.getByText('No signed release yet').first().waitFor({ timeout: 4000 });
    const looks = new Set();
    for (const [name, want] of COPIES) {
      const r = await copyOn(page, name);
      if (!want.test(r.clip)) fail(where, `${name}: copied ${JSON.stringify(r.clip)}`);
      if (r.label !== 'Copied') fail(where, `${name}: says "${r.label}" after copying`);
      looks.add(r.look);
    }
    if (looks.size !== 1) fail(where, `copied buttons look different: ${[...looks].join(' | ')}`);
    await ctx.close();
  }
  // A blocker that answers "copied" without copying anything.
  {
    const where = 'download copy blocked';
    const ctx = await browser.newContext({ viewport: VIEWPORTS.mobile, ...perms });
    await ctx.addInitScript(() => {
      const blocked = (text) => /\biex\b/.test(String(text ?? ''));
      const writeText = navigator.clipboard.writeText.bind(navigator.clipboard);
      navigator.clipboard.writeText = (text) => (blocked(text) ? Promise.resolve() : writeText(text));
      const exec = document.execCommand.bind(document);
      document.execCommand = (cmd, ...rest) => (cmd === 'copy' && blocked(getSelection()?.toString()) ? true : exec(cmd, ...rest));
    });
    const page = await ctx.newPage();
    await watch(page, where);
    await page.goto(ORIGIN + '/download/');
    await page.getByText('No signed release yet').first().waitFor({ timeout: 4000 });
    const r = await copyOn(page, 'Copy the Windows install command');
    if (r.clip !== 'unchanged') fail(where, `the clipboard changed to ${JSON.stringify(r.clip)}`);
    if (!/^Press (Ctrl\+C|⌘C)$/.test(r.label ?? '')) fail(where, `the button says "${r.label}", not which keys copy the text`);
    if (!COPIES[1][1].test(r.selected)) fail(where, `the command is not selected (${JSON.stringify(r.selected)})`);
    if (!(await page.locator('#install .copy-note').isVisible())) fail(where, 'no note says the copy was blocked');
    const ok = await copyOn(page, 'Copy the macOS and Linux install command');
    if (!COPIES[0][1].test(ok.clip) || ok.label !== 'Copied') fail(where, 'a command the blocker allows was not copied');
    await page.screenshot({ path: join(shotsDir, 'download-copy-blocked-mobile.png') });
    await ctx.close();
  }
  // Without the copy command, the clipboard API copies instead.
  {
    const where = 'download copy fallback';
    const ctx = await browser.newContext({ viewport: VIEWPORTS.desktop, ...perms });
    await ctx.addInitScript(() => { document.execCommand = () => false; });
    const page = await ctx.newPage();
    await watch(page, where);
    await page.goto(ORIGIN + '/download/');
    await page.getByText('No signed release yet').first().waitFor({ timeout: 4000 });
    const r = await copyOn(page, 'Copy the desktop build command');
    if (!COPIES[3][1].test(r.clip) || r.label !== 'Copied') fail(where, `copied ${JSON.stringify(r.clip)}, says "${r.label}"`);
    await ctx.close();
  }
}

/* ---------- screenshots to look at ---------- */
async function shots(browser) {
  console.log('• screenshots');
  const set = [
    ['/', 'desktop', 'light', true], ['/', 'desktop', 'dark', true], ['/', 'mobile', 'light', true], ['/', 'desktop', 'light', false],
    ['/download/', 'desktop', 'light', false], ['/verify/', 'desktop', 'light', true], ['/protocol/', 'mobile', 'dark', true],
    ['/relay/', 'desktop', 'dark', true], ['/communities/', 'desktop', 'light', true], ['/protocol/', 'desktop', 'light', true],
    ['/protocol/', 'mobile', 'light', true], ['/404.html', 'mobile', 'light', true], ['/relay/', 'mobile', 'light', true],
    ['/communities/', 'mobile', 'dark', true], ['/', 'small', 'light', true], ['/download/', 'small', 'light', true],
  ];
  for (const [path, vp, colorScheme, js] of set) {
    const ctx = await browser.newContext({ viewport: VIEWPORTS[vp], colorScheme, javaScriptEnabled: js, reducedMotion: 'reduce' });
    const page = await ctx.newPage();
    await page.goto(ORIGIN + path);
    await page.evaluate(() => document.fonts.ready);
    await sleep(300);
    const name = `${path.replace(/\//g, '_').replace(/^_|_$/g, '') || 'home'}-${vp}-${colorScheme}${js ? '' : '-nojs'}.png`;
    await page.screenshot({ path: join(shotsDir, name), fullPage: true });
    await ctx.close();
  }
  // the hero mid-animation and at rest, with motion
  const ctx = await browser.newContext({ viewport: VIEWPORTS.desktop });
  const page = await ctx.newPage();
  await page.goto(ORIGIN + '/');
  await sleep(2500);
  await page.screenshot({ path: join(shotsDir, 'home-hero-motion.png') });
  await ctx.close();
}

/* ---------- run ---------- */
mkdirSync(shotsDir, { recursive: true });
let browser;
try {
  await waitForServer();
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  if (PUBLISHED) {
    const newContext = browser.newContext.bind(browser);
    browser.newContext = async (options) => {
      const ctx = await newContext(options);
      await ctx.route(`${PUBLISHED}/manifest.json`, (r) => r.fulfill({ body: PLACEHOLDER, contentType: 'application/json' }));
      return ctx;
    };
  }
  if (args.includes('--playground')) {
    await playgroundPass(browser);
  } else if (args.includes('--download')) {
    await download(browser);
  } else if (!shotsOnly) {
    await matrix(browser);
    await wikiPass(browser);
    await playgroundPass(browser);
    await demos(browser);
    await download(browser);
  }
  if (!args.includes('--playground') && !args.includes('--download')) await shots(browser);
} catch (e) {
  fail('runner', e.stack || String(e));
} finally {
  await browser?.close();
  server.kill();
}
console.log(failures.length ? `\n${failures.length} problem(s).` : `\nAll good. Screenshots in ${shotsDir}`);
process.exit(failures.length ? 1 : 0);
