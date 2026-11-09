// Production CSP and copy controls in Firefox and Chromium. Build the site first:
//   pnpm --filter @hoi/site build
//   node apps/site/scripts/e2e-csp.mjs [--browser chromium|firefox]
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, firefox } from 'playwright';
import { serveStatic } from '../../../scripts/serve-static.mjs';

const site = fileURLToPath(new URL('..', import.meta.url));
const dist = join(site, 'dist');
const publication = JSON.parse(readFileSync(join(site, '../../publication.json'), 'utf8'));
const manifest = publication.downloads ? `${publication.downloads.replace(/\/+$/, '')}/manifest.json` : '';
const placeholder = readFileSync(join(site, 'public/releases/manifest.json'));
const args = process.argv.slice(2);
const selected = args.includes('--browser') ? args[args.indexOf('--browser') + 1] : null;
assert.ok(!args.length || (args.length === 2 && args[0] === '--browser' && ['chromium', 'firefox'].includes(selected)), 'Use --browser chromium|firefox');

function htmlPages(dir, prefix = '/') {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const name = prefix + entry.name;
    if (entry.isDirectory()) return htmlPages(join(dir, entry.name), name + '/');
    return entry.name.endsWith('.html') ? [name.replace(/index\.html$/, '')] : [];
  }).sort();
}
const paths = htmlPages(dist);
assert.ok(paths.includes('/') && paths.includes('/download/') && paths.includes('/404.html'), 'Build the complete production site first');
const server = await serveStatic({ root: dist });

async function offline(context, failures) {
  await context.route('**/*', async (route) => {
    const url = route.request().url();
    if (manifest && url === manifest) return route.fulfill({ body: placeholder, contentType: 'application/json' });
    if (new URL(url).origin === server.origin) return route.continue();
    failures.push(`Unexpected external request: ${url}`);
    await route.abort();
  });
}

async function positiveControl(browser) {
  const context = await browser.newContext();
  const failures = [];
  try {
    await offline(context, failures);
    await context.addInitScript(() => {
      window.cspEvents = [];
      document.addEventListener('securitypolicyviolation', (e) => window.cspEvents.push(e.effectiveDirective));
    });
    const page = await context.newPage();
    await page.goto(server.origin + '/verify/');
    assert.deepEqual(await page.evaluate(() => window.cspEvents), []);
    // A separate page proves that enforcement and the listener are active without polluting the audit.
    await page.locator('h1').evaluate((el) => el.setAttribute('style', 'color: rgb(1, 2, 3)'));
    await page.waitForFunction(() => window.cspEvents.includes('style-src-attr'));
    assert.notEqual(await page.locator('h1').evaluate((el) => getComputedStyle(el).color), 'rgb(1, 2, 3)');
    assert.deepEqual(failures, []);
  } finally {
    await context.close();
  }
}

async function scrollThrough(page) {
  await page.evaluate(async () => {
    // Snapshot the height: animations can change it while scrolling.
    const height = document.documentElement.scrollHeight;
    const step = Math.max(200, Math.floor(innerHeight * 0.75));
    for (let y = 0; y < height; y += step) {
      window.scrollTo(0, y);
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    window.scrollTo(0, 0);
  });
}

const COPY = 'button[data-copy], button.wk-copy';
async function copyFeedback(page, button, denied) {
  await button.evaluate((btn, label) => new Promise((resolve, reject) => {
    if (btn.textContent === label) return resolve();
    const observer = new MutationObserver(() => {
      if (btn.textContent === label) { clearTimeout(timeout); observer.disconnect(); resolve(); }
    });
    const timeout = setTimeout(() => { observer.disconnect(); reject(new Error(`Expected ${label}`)); }, 2000);
    observer.observe(btn, { childList: true, subtree: true, characterData: true });
  }), denied ? 'Selected' : 'Copied');
  const announcement = denied ? 'Selected. Press Control C or Command C to copy.' : 'Copied to the clipboard';
  await page.waitForFunction((text) => document.querySelector('.sr-only[role="status"][aria-live="polite"]')?.textContent === text, announcement);
}

async function checkCopy(page, button, denied, where) {
  const before = await button.evaluate((btn) => {
    const wiki = btn.matches('.wk-copy');
    const box = btn.closest(wiki ? '.wk-code' : '.code');
    const source = (wiki ? box?.querySelector('pre code') : null) ?? box?.querySelector('pre') ?? btn.parentElement?.querySelector('code');
    if (!source) throw new Error('Copy control has no visible source');
    // Walk text nodes independently of the implementation; copying must not edit the display.
    const walker = document.createTreeWalker(source, NodeFilter.SHOW_TEXT, {
      acceptNode: (node) => !wiki && node.parentElement?.closest('.c') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT,
    });
    let text = '';
    while (walker.nextNode()) text += walker.currentNode.textContent;
    const expected = btn.dataset.copyText || (wiki ? text.replace(/\n$/, '') : text.split('\n').map((line) => line.trimEnd()).filter((line) => line.trim()).join('\n'));
    return { expected, selected: source.textContent, html: source.innerHTML };
  });
  assert.ok(before.expected, `${where}: nonempty copy source`);
  await page.evaluate((deny) => { window.denyClipboard = deny; window.copiedText = []; }, denied);
  await button.click();
  await copyFeedback(page, button, denied);
  assert.deepEqual(await page.evaluate(() => window.copiedText), [before.expected], `${where}: exact clipboard text`);
  // Firefox serializes selections with Windows line endings; DOM textContent keeps LF.
  if (denied) assert.equal(await page.evaluate(() => window.getSelection()?.toString().replace(/\r\n/g, '\n')), before.selected.replace(/\r\n/g, '\n'), `${where}: fallback selects source`);
  const after = await button.evaluate((btn) => {
    const wiki = btn.matches('.wk-copy');
    const box = btn.closest(wiki ? '.wk-code' : '.code');
    return ((wiki ? box?.querySelector('pre code') : null) ?? box?.querySelector('pre') ?? btn.parentElement?.querySelector('code'))?.innerHTML;
  });
  assert.equal(after, before.html, `${where}: copying preserves displayed markup`);
}

async function interactions(page, path) {
  if (path === '/') {
    await page.locator('#signBtn').click();
    await page.locator('#resetBtn').waitFor({ state: 'visible' });
    await page.locator('#resetBtn').click();
    await page.locator('#floodBtn').click();
    await page.waitForFunction(() => Number(getComputedStyle(document.querySelector('#badge')).opacity) > 0.99);
    const chip = page.locator('#zone .chip').first();
    await chip.scrollIntoViewIfNeeded();
    await page.waitForTimeout(900);
    const box = await chip.boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 100, box.y + box.height / 2 + 50, { steps: 8 });
    await page.mouse.up();
  } else if (path === '/relay/') {
    await page.locator('#relaySw').click();
    await page.locator('#noticeOn').click();
    await page.locator('#relaySw').click();
  } else if (path === '/wiki/playground/') {
    await page.locator('[data-pg-play]').click();
    await page.waitForTimeout(1000);
    await page.locator('[data-pg-reset]').click();
    await page.locator('[data-pg-zin]').click();
    await page.locator('[data-pg-fit]').click();
  }
  const buttons = page.locator('[data-demo] button');
  for (let i = 0; i < await buttons.count(); i++) {
    const button = buttons.nth(i);
    if (await button.isVisible() && await button.isEnabled()) {
      await button.click();
      await page.waitForTimeout(300);
    }
  }
}

async function auditedPage(browser, mockClipboard) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' });
  const failures = [];
  await offline(context, failures);
  await context.exposeFunction('recordCsp', (event) => failures.push(`CSP: ${JSON.stringify(event)}`));
  await context.addInitScript((mock) => {
    document.addEventListener('securitypolicyviolation', (e) => window.recordCsp({
      path: location.pathname, directive: e.effectiveDirective, blockedURI: e.blockedURI,
      source: e.sourceFile, line: e.lineNumber,
    }));
    if (!mock) return;
    // Deterministic success and denial, independent of OS clipboard permissions.
    window.copiedText = [];
    window.denyClipboard = false;
    Object.defineProperty(navigator.clipboard, 'writeText', { configurable: true, value: async (text) => {
      window.copiedText.push(text);
      if (window.denyClipboard) throw new DOMException('Clipboard denied', 'NotAllowedError');
    } });
  }, mockClipboard);
  const page = await context.newPage();
  page.on('console', (message) => { if (message.type() === 'error') failures.push(`${page.url()}: ${message.text()}`); });
  page.on('pageerror', (error) => failures.push(`${page.url()}: ${error.message}`));
  page.on('requestfailed', (request) => failures.push(`${request.url()}: ${request.failure()?.errorText}`));
  page.on('response', (response) => { if (response.status() >= 400) failures.push(`HTTP ${response.status()}: ${response.url()}`); });
  return { context, page, failures };
}

async function nativeCopy(browser) {
  const { context, page, failures } = await auditedPage(browser, false);
  try {
    if (browser.browserType().name() === 'chromium') await context.grantPermissions(['clipboard-write'], { origin: server.origin });
    await page.goto(server.origin + '/download/', { waitUntil: 'networkidle' });
    await page.bringToFront();
    const buttons = page.locator('#install button[data-copy]');
    assert.equal(await buttons.count(), 2, 'Both install-command copy buttons exist');
    for (const button of await buttons.all()) {
      assert.ok(await button.isVisible(), 'Install-command copy button is visible');
      await button.click();
      await copyFeedback(page, button, false);
    }
    await page.waitForTimeout(100);
    assert.deepEqual(failures, [], `${browser.browserType().name()}: native install-command copying`);
  } finally {
    await context.close();
  }
}

try {
  for (const engine of [chromium, firefox].filter((engine) => !selected || engine.name() === selected)) {
    const browser = await engine.launch({ executablePath: process.env[`${engine.name().toUpperCase()}_PATH`] || undefined });
    try {
      await positiveControl(browser);
      await nativeCopy(browser);
      const { context, page, failures } = await auditedPage(browser, true);
      let copied = 0, denied = 0;
      for (const path of paths) {
        await page.goto(server.origin + path, { waitUntil: 'networkidle' });
        const meta = page.locator('meta[http-equiv="Content-Security-Policy"]');
        assert.equal(await meta.count(), 1, `${path}: production CSP meta is present`);
        const policy = await meta.getAttribute('content');
        assert.ok(policy.split(';').some((directive) => directive.trim() === "style-src 'self'"), `${path}: strict style-src`);
        assert.doesNotMatch(policy, /'unsafe-inline'|'unsafe-eval'/, `${path}: no broad policy bypass`);
        await scrollThrough(page);
        const buttons = page.locator(COPY);
        if (path === '/wiki/records/') assert.ok(await page.locator('button.wk-copy:visible').count() > 0, 'Wiki copy controls are visible');
        for (let i = 0; i < await buttons.count(); i++) {
          const button = buttons.nth(i);
          if (!await button.isVisible()) continue;
          const where = `${engine.name()} ${path} copy ${i + 1}`;
          await checkCopy(page, button, false, where);
          copied++;
          if (['/download/', '/verify/', '/wiki/records/'].includes(path)) {
            await checkCopy(page, button, true, where);
            denied++;
          }
        }
        await interactions(page, path);
        await page.waitForTimeout(100);
        assert.deepEqual(failures, [], `${engine.name()} ${path}`);
        console.log(`PASS ${engine.name()} ${path}`);
      }
      await context.close();
      assert.deepEqual(failures, []);
      console.log(`PASS ${engine.name()}: ${paths.length} pages, 2 native install copies, ${copied} mocked copies, ${denied} clipboard fallbacks; CSP positive control passed.`);
    } finally {
      await browser.close();
    }
  }
} finally {
  await server.close();
}
