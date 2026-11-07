// Isolated mock-UI regression: settings at narrow/wide widths and bounded profile cropping.
// Run with node apps/web/scripts/e2e-qol.mjs; CHROMIUM_PATH selects an installed browser.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { consentAccepted } from '../../../scripts/browser-vault.mjs';

const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const shots = resolve(web, '.shots/qol');
mkdirSync(shots, { recursive: true });
const port = 5700 + Math.floor(Math.random() * 200);
const base = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, [resolve(web, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: web, env: { ...process.env, VITE_HOI_BACKEND: 'mock', VITE_HOI_REGISTRY_URL: '' }, stdio: ['ignore', 'pipe', 'pipe'],
});
let serverOutput = '';
server.stdout.on('data', b => { serverOutput += b; });
server.stderr.on('data', b => { serverOutput += b; });
let browser;
try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(base)).ok) { ready = true; break; } } catch { /* Starting. */ }
    if (server.exitCode !== null) break;
    await new Promise(r => setTimeout(r, 100));
  }
  assert(ready, `UI server did not start: ${serverOutput}`);
  const chrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || (existsSync(chrome) ? chrome : undefined) });
  for (const config of [
    { name: 'browser-wide', width: 1280, colorScheme: 'light', query: '' },
    { name: 'desktop-medium', width: 1000, colorScheme: 'dark', query: '&platform=desktop&full' },
    { name: 'phone', width: 390, colorScheme: 'light', query: '' },
    { name: 'phone-small', width: 360, colorScheme: 'dark', query: '' },
  ]) {
    const context = await browser.newContext({ viewport: { width: config.width, height: 900 }, colorScheme: config.colorScheme, reducedMotion: 'reduce' });
    await consentAccepted(context);
    const page = await context.newPage();
    page.setDefaultTimeout(10_000);
    const failures = [];
    page.on('pageerror', error => failures.push(error.message));
    page.on('console', message => { if (message.type() === 'error') failures.push(message.text()); });
    await page.goto(`${base}/?me=lena${config.query}#/you`);
    await page.getByRole('heading', { name: /Hey lena/ }).waitFor();
    assert.equal(await page.locator('.avatar-initials').count(), 2, 'Only the header and navigation show account badges');
    assert.equal(await page.locator('.idhead .avatar-initials').textContent(), 'LE');
    assert.equal(await page.locator('.me .avatar-initials').textContent(), 'LE');
    assert.equal(await page.locator('.profile-picture .profile-avatar').count(), 0, 'The picture editor does not duplicate the account preview');
    const pattern = await page.locator('.idhead .identicon .dot').evaluateAll(dots => dots.map(dot => dot.outerHTML));
    await page.getByRole('radio', { name: 'Sky', exact: true }).check();
    await page.locator('.idhead .identicon .bg-sky').waitFor();
    await page.locator('.me .identicon .bg-sky').waitFor();
    assert.deepEqual(await page.locator('.idhead .identicon .dot').evaluateAll(dots => dots.map(dot => dot.outerHTML)), pattern, 'A color choice preserves the key pattern');
    await page.getByRole('heading', { name: 'Open connections', exact: true }).waitFor();
    assert.equal(await page.locator('#y-block').isVisible(), false, 'Fingerprint imports start collapsed');
    await page.getByText('Import a trusted fingerprint list', { exact: true }).click();
    assert.equal(await page.locator('#y-block').isVisible(), true);
    assert(!String(await page.locator('#y-block').getAttribute('accept')).includes('image/'));
    await page.locator('details.diagnostics > summary').click();
    await page.getByRole('heading', { name: 'Last catch-up', exact: true }).waitFor();
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
    assert(overflow <= 0, `${config.name} overflowed by ${overflow}px`);
    const descriptionWidth = await page.locator('.set', { has: page.locator('#y-pair') }).locator('.h').evaluate(el => el.getBoundingClientRect().width);
    assert(descriptionWidth >= 220, `${config.name} pairing description was squeezed to ${descriptionWidth}px`);
    // A row whose value cell comes from a child component (Labelers) lines up under its description.
    const labelers = page.locator('.set', { has: page.locator('#lb-key') });
    const [hLeft, vLeft, vWidth] = await labelers.evaluate(row => {
      const h = row.querySelector('.h').getBoundingClientRect();
      const v = row.querySelector('.v').getBoundingClientRect();
      return [h.left, v.left, v.width];
    });
    assert(Math.abs(hLeft - vLeft) <= 1, `${config.name} labelers sit at ${vLeft}px, their description at ${hLeft}px`);
    assert(vWidth >= 220, `${config.name} labelers were squeezed to ${vWidth}px`);
    await page.screenshot({ path: resolve(shots, `${config.name}-settings.png`), fullPage: true });

    // Generate a wide input locally: cropping must fill the circular preview without stretching.
    const encoded = await page.evaluate(() => {
      const canvas = document.createElement('canvas'); canvas.width = 800; canvas.height = 400;
      const c = canvas.getContext('2d');
      c.fillStyle = '#f26b50'; c.fillRect(0, 0, 400, 400);
      c.fillStyle = '#3159db'; c.fillRect(400, 0, 400, 400);
      c.fillStyle = '#ffd24d'; c.beginPath(); c.arc(400, 200, 100, 0, 2 * Math.PI); c.fill();
      return canvas.toDataURL('image/png').split(',')[1];
    });
    await page.getByLabel('Choose profile picture', { exact: true }).setInputFiles({ name: 'test-portrait.png', mimeType: 'image/png', buffer: Buffer.from(encoded, 'base64') });
    await page.getByRole('heading', { name: 'Frame your picture' }).waitFor();
    await page.getByRole('slider', { name: 'Zoom', exact: true }).fill('1.6');
    await page.getByRole('slider', { name: 'Horizontal position' }).fill('0.5');
    await page.getByRole('slider', { name: 'Vertical position' }).fill('-0.25');
    await page.screenshot({ path: resolve(shots, `${config.name}-crop.png`) });
    await page.getByRole('button', { name: 'Use this picture', exact: true }).click();
    const photo = page.locator('.idhead img');
    await photo.waitFor();
    const uploaded = await photo.evaluate(async img => { await img.decode(); return { width: img.naturalWidth, height: img.naturalHeight, src: img.src }; });
    assert(uploaded.width <= 128 && uploaded.width === uploaded.height, 'Saved profile must be a small square');
    assert(Buffer.from(uploaded.src.split(',')[1], 'base64').length <= 4096, 'Saved photo exceeds 4 KiB');
    assert.equal(await page.locator('.me img').getAttribute('src'), uploaded.src, 'Navigation uses the selected photo');
    assert.equal(await page.locator('.avatar-initials').count(), 2, 'A custom photo keeps both private account badges');
    assert.equal(await page.locator('.profile-picture img').count(), 0, 'Uploading does not add a duplicate preview');
    await page.getByRole('button', { name: 'Use key picture', exact: true }).click();
    await photo.waitFor({ state: 'detached' });
    await page.locator('.idhead .identicon .bg-sky').waitFor();
    await page.getByRole('button', { name: 'Change name', exact: true }).click();
    await page.getByLabel('Your name', { exact: true }).fill('Lena River');
    await page.locator('.namef').getByRole('button', { name: 'Save', exact: true }).click();
    await page.getByRole('heading', { name: /Hey Lena River/ }).waitFor();
    assert.equal(await page.locator('.idhead .avatar-initials').textContent(), 'LR');
    assert.equal(await page.locator('.me .avatar-initials').textContent(), 'LR');
    await page.getByRole('radio', { name: 'From key', exact: true }).check();
    await page.waitForFunction(async () => !(await import('/src/lib/app.svelte.ts')).app.me.avatarColor);
    await page.screenshot({ path: resolve(shots, `${config.name}-profile.png`), fullPage: true });
    await page.evaluate(() => scrollTo(0, 0));
    await page.screenshot({ path: resolve(shots, `${config.name}-account.png`) });
    // Existing public rows never acquire the private account badge.
    await page.goto(`${base}/?me=lena${config.query}#/`);
    await page.locator('.row .author .profile-avatar').first().waitFor();
    assert.equal(await page.locator('.row .avatar-initials').count(), 0);
    assert.deepEqual(failures, [], `${config.name} browser errors`);
    await context.close();
    console.log(`PASS ${config.name}: settings, private initials, color, crop, bounded upload, key-picture reset`);
  }
} finally {
  await browser?.close();
  server.kill();
}
