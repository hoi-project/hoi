// Built-site integration tests. The browser API is mocked; this does not benchmark Gemini Nano.
// Build first, then run: node apps/site/scripts/e2e-assistant.mjs
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { chromium } from 'playwright';

const site = resolve(import.meta.dirname, '..');
const port = 4329;
const origin = 'http://localhost:' + port;
const artifacts = resolve(site, '../../../work/wiki-assistant');
mkdirSync(artifacts, { recursive: true });
const server = spawn(process.execPath, [resolve(site, 'node_modules/vite/bin/vite.js'), 'preview', '--host', 'localhost', '--port', String(port), '--strictPort'], { cwd: site, stdio: ['ignore', 'pipe', 'pipe'] });
let log = '';
let startupError;
server.on('error', (error) => { startupError = error; });
server.stdout.on('data', (data) => { log += data; });
server.stderr.on('data', (data) => { log += data; });
let browser;
const errors = [];
async function mock(page, mode) {
  await page.addInitScript((mode) => {
    window.__aiCalls = { create: 0, destroy: 0, prompts: [], activation: [] };
    window.__aiMode = mode;
    const api = {
      async availability() {
        if (mode === 'throws') throw new Error('Blocked by policy');
        return mode === 'unavailable' ? 'unavailable' : mode === 'downloadable' ? 'downloadable' : 'available';
      },
      async create(options) {
        window.__aiCalls.create++;
        window.__aiCalls.activation.push(navigator.userActivation.isActive);
        if (window.__aiMode === 'create-error') throw new Error('Cannot create');
        const monitor = new EventTarget();
        options.monitor(monitor);
        monitor.dispatchEvent(Object.assign(new Event('downloadprogress'), { loaded: 0.5 }));
        return {
          contextWindow: 32768, contextUsage: 300,
          async measureContextUsage() { return 4000; },
          async prompt(raw, { signal }) {
            const data = JSON.parse(raw);
            window.__aiCalls.prompts.push(data);
            if (window.__aiMode === 'slow') {
              await new Promise((resolve, reject) => {
                if (signal.aborted) reject(signal.reason);
                else signal.addEventListener('abort', () => reject(signal.reason), { once: true });
              });
            }
            if (window.__aiMode === 'invalid') return '{broken';
            const here = data.SOURCES.find((s) => s.currentSection);
            const elsewhere = data.SOURCES.find((s) => !s.currentSection && s.page !== here.page);
            const samePage = data.SOURCES.find((s) => !s.currentSection && s.page === here.page);
            const next = window.__aiMode === 'same-page' ? samePage : elsewhere;
            const answer = data.CONTINUATION
              ? 'Here you can see how this section connects to your earlier question about recovery. Follow the documented steps here to understand the next part.'
              : 'The recovery line lets you return to your identity. This section explains what it protects and how it connects to the vault on your device.';
            return JSON.stringify({
              status: 'answered',
              answer: window.__aiMode === 'html' ? '<img src=x onerror=alert(1)> is plain reference text.' : answer,
              sources: [here.number],
              next: next ? [{ source: next.number, reason: 'See how this part is used and what comes next.', question: 'How does this section help with the recovery question?' }] : [],
            });
          },
          destroy() { window.__aiCalls.destroy++; },
        };
      },
    };
    Object.defineProperty(window, 'LanguageModel', { configurable: true, value: mode === 'missing' ? undefined : api });
  }, mode);
}
async function pageFor(mode, width = 1280) {
  const context = await browser.newContext({ viewport: { width, height: 960 }, reducedMotion: 'reduce' });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()); });
  page.on('request', (request) => {
    assert.equal(new URL(request.url()).origin, origin, 'No third-party requests');
  });
  await mock(page, mode);
  await page.goto(origin + '/wiki/identity/#recovery-line');
  await page.locator('.wk-dlg').waitFor({ state: 'attached' });
  return { page, context };
}
const opener = (page) => page.locator('#recovery-line > h2 .wk-help-open');
async function ready(page) {
  await page.locator('.wk-help-status').filter({ hasText: 'Explanation ready.' }).waitFor();
}
async function ask(page, text = 'How does recovery work in the web app vault?') {
  await page.locator('#wk-help-question').fill(text);
  await page.getByRole('button', { name: 'Ask', exact: true }).click();
  await ready(page);
}
try {
  for (let i = 0; i < 100; i++) {
    if (startupError) throw startupError;
    try { if ((await fetch(origin)).ok) break; } catch {}
    if (i === 99) throw new Error(log);
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || (existsSync('/usr/bin/chromium') ? '/usr/bin/chromium' : undefined), headless: true, args: ['--no-sandbox'] });
  for (const mode of ['missing', 'unavailable', 'throws']) {
    const { page, context } = await pageFor(mode);
    assert.equal(await page.locator('.wk-help-open, .wk-help').count(), 0);
    assert.equal(await page.evaluate(() => window.__aiCalls.create), 0);
    assert.equal(await page.evaluate(() => performance.getEntriesByType('resource').some((r) => r.name.includes('wiki-passages'))), false);
    await context.close();
  }
  console.log('✓ Unsupported devices and policy failures show no AI UI or corpus downloads.');

  {
    const { page, context } = await pageFor('available');
    await opener(page).waitFor();
    assert.equal(await page.evaluate(() => window.__aiCalls.create), 0);
    assert.equal(await page.locator('.wk-help').count(), 0);
    await opener(page).click();
    await page.locator('.wk-help-options button').first().waitFor();
    assert.equal(await page.evaluate(() => window.__aiCalls.create), 0);
    await page.screenshot({ path: resolve(artifacts, 'options.png') });
    await ask(page);
    await page.evaluate(readFileSync(resolve(site, '../../node_modules/axe-core/axe.min.js'), 'utf8'));
    const a11y = await page.evaluate(async () => (await window.axe.run(document.querySelector('.wk-help'), {
      runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21aa'] },
    })).violations.map((v) => ({ id: v.id, description: v.description })));
    assert.deepEqual(a11y, [], 'Explanation card accessibility');
    await page.screenshot({ path: resolve(artifacts, 'answer.png') });
    const destination = await page.locator('.wk-help-next').first().getAttribute('href');
    assert.ok(destination && !destination.startsWith('/wiki/identity/'), 'Cross-page destination must be grounded in retrieved docs');
    await page.locator('.wk-help-next').first().click();
    await page.waitForURL(origin + destination);
    await ready(page);
    assert.match(await page.locator('.wk-help-intro').textContent(), /Continuing from/);
    assert.match(await page.locator('.wk-help-answer').textContent(), /earlier question/);
    assert.equal(await page.evaluate(() => sessionStorage.getItem('hoi.wiki.explanation')), null);
    assert.equal(new URL(page.url()).search, '');
    assert.ok((await page.evaluate(() => window.__aiCalls.prompts[0])).CONTINUATION.originalQuestion.includes('vault'));
    await page.screenshot({ path: resolve(artifacts, 'continuation.png') });
    await ask(page, 'Why is that needed here?');
    assert.equal((await page.evaluate(() => window.__aiCalls.prompts.at(-1))).RECENT_CONVERSATION.length, 1);
    await page.locator('#wk-help-question').press('Escape');
    assert.equal(await page.locator('.wk-help').count(), 0);
    assert.equal(await page.locator('.wk-help-open:focus').count(), 1);
    assert.equal(await page.evaluate(() => window.__aiCalls.create), await page.evaluate(() => window.__aiCalls.destroy));
    await context.close();
  }
  console.log('✓ Questions, citations, cross-page continuation, follow-ups, cleanup and keyboard close.');

  {
    const { page, context } = await pageFor('same-page');
    await opener(page).click();
    await ask(page, 'How does the recovery line connect to generations and forgetting the line?');
    const destination = await page.locator('.wk-help-next').first().getAttribute('href');
    assert.ok(destination?.startsWith('/wiki/identity/#'));
    await page.locator('.wk-help-next').first().click();
    await ready(page);
    assert.equal(await page.locator('.wk-help').count(), 1);
    assert.equal(await page.locator('.wk-help-open[aria-expanded="true"]').count(), 1);
    assert.equal(await page.evaluate(() => sessionStorage.getItem('hoi.wiki.explanation')), null);
    assert.ok((await page.evaluate(() => window.__aiCalls.prompts.at(-1))).CONTINUATION);
    await page.goBack();
    await page.locator('.wk-help').waitFor({ state: 'detached' });
    await context.close();
  }
  console.log('✓ Same-page connections move the single inline card and keep context in memory.');

  {
    const { page, context } = await pageFor('available');
    await page.evaluate(() => sessionStorage.setItem('hoi.wiki.explanation', JSON.stringify({
      version: 'old-content', original: 'How does recovery work?', question: 'What next?', answer: 'An earlier explanation.',
      from: '/wiki/web-app/#vault-errors', fromTitle: 'Errors and recovery',
      to: '/wiki/identity/#recovery-line', at: Date.now(),
    })));
    await page.reload();
    await page.getByRole('status').filter({ hasText: 'documentation changed' }).waitFor();
    assert.equal(await page.evaluate(() => window.__aiCalls.create), 0);
    assert.equal(await page.evaluate(() => sessionStorage.getItem('hoi.wiki.explanation')), null);
    await context.close();
  }
  console.log('✓ Changed documentation discards old context without starting inference.');

  {
    const { page, context } = await pageFor('downloadable', 390);
    await opener(page).click();
    assert.match(await page.locator('.wk-help-note').textContent(), /model download/);
    await page.locator('.wk-help-options button').first().click();
    await ready(page);
    assert.equal(await page.evaluate(() => window.__aiCalls.activation[0]), true);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    const undersized = await page.locator('.wk-help button').evaluateAll((els) => els.filter((e) => !e.hidden && e.getBoundingClientRect().height < 44).length);
    assert.equal(undersized, 0);
    await page.screenshot({ path: resolve(artifacts, 'narrow.png') });
    await context.close();
  }
  console.log('✓ Download setup starts on a user gesture; narrow layout keeps usable controls.');

  {
    const { page, context } = await pageFor('slow');
    await opener(page).click();
    await page.locator('.wk-help-options button').first().click();
    await page.waitForFunction(() => window.__aiCalls.prompts.length > 0);
    await page.getByRole('button', { name: 'Stop', exact: true }).click();
    await page.getByRole('status').filter({ hasText: 'Stopped.' }).waitFor();
    assert.equal(await page.locator('.wk-help-answer').count(), 0);
    await page.evaluate(() => { window.__aiMode = 'invalid'; });
    await page.getByRole('button', { name: 'Ask', exact: true }).click();
    await page.getByRole('status').filter({ hasText: 'Couldn’t prepare' }).waitFor();
    await page.evaluate(() => { window.__aiMode = 'html'; });
    await ask(page);
    assert.equal(await page.locator('.wk-help-answer img').count(), 0);
    assert.match(await page.locator('.wk-help-answer').textContent(), /<img/);
    await page.evaluate(() => { window.__aiMode = 'create-error'; });
    await page.locator('.wk-help-options button').first().click();
    await page.getByRole('status').filter({ hasText: 'Couldn’t prepare' }).waitFor();
    assert.equal(await page.getByRole('button', { name: 'Ask', exact: true }).isEnabled(), true);
    await context.close();
  }
  console.log('✓ Cancellation, malformed output, safe text rendering and retryable setup failures.');
  assert.deepEqual(errors, []);
  console.log('PASS: built wiki explanation flow (mocked model). Screenshots: ' + artifacts);
} finally {
  await browser?.close();
  server.kill();
}
