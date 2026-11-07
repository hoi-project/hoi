// Registry navigation and search privacy through the actual UI, with signed registry fixtures.
// Uses the existing mock core and intercepts only registry requests. No external service needed.
// node scripts/e2e-discovery.mjs [--shots test-results/discovery-qa]
import assert from 'node:assert/strict';
import { generateKeyPairSync, sign } from 'node:crypto';
import { readFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { consentAccepted } from './browser-vault.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const release = JSON.parse(readFileSync(join(root, 'release.json'), 'utf8'));
const port = 5341;
const registry = 'http://127.0.0.1:5342';
const args = process.argv.slice(2);
const shots = args.includes('--shots') ? resolve(args[args.indexOf('--shots') + 1]) : null;
if (shots) mkdirSync(shots, { recursive: true });
const key = generateKeyPairSync('ed25519');
const community = key.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
const proof = { version: 2, protocolVersion: release.protocolVersion, indexVersion: release.indexVersion,
  signature: sign(null, Buffer.from(`hoi community index v2\nbikes\n${community}\n${release.protocolVersion}\n${release.indexVersion}`), key.privateKey).toString('hex') };
const entry = { name: 'bikes', community, proof, protocolVersion: release.protocolVersion, indexVersion: release.indexVersion, compatible: true, indexed: true };

let browser;
process.env.VITE_HOI_BACKEND = 'mock';
process.env.VITE_HOI_REGISTRY_URL = registry;
const server = await createServer({
  root: join(root, 'apps/web'), logLevel: 'error',
  server: { host: '127.0.0.1', port, strictPort: true, hmr: false },
});
try {
  // Await the listening socket; Vite's coloured CLI URL is not a readiness protocol.
  await server.listen();
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce' });
  await consentAccepted(page.context());
  page.setDefaultTimeout(15_000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const lookups = [];
  let unavailable = false;
  await page.route(`${registry}/**`, async route => {
    const url = new URL(route.request().url());
    const headers = { 'Access-Control-Allow-Origin': '*', 'Content-Type': 'application/json' };
    if (url.pathname === '/release.json') return route.fulfill({ headers, body: JSON.stringify(release) });
    lookups.push(url);
    if (unavailable) return route.fulfill({ status: 503, headers, body: '{}' });
    const prefix = url.searchParams.get('prefix') ?? '';
    return route.fulfill({ headers, body: JSON.stringify('bikes'.startsWith(prefix) ? [entry] : []) });
  });
  await page.goto(`http://127.0.0.1:${port}/?me=discovery-check#/search?q=private%20words`);
  await page.getByRole('heading', { name: 'Search', exact: true }).waitFor();
  await page.getByText('Nothing for “private words”.', { exact: true }).waitFor();
  assert.equal(lookups.length, 0, 'Post searches must never contact the registry');
  await page.locator('#search-scope').selectOption('communities');
  await page.locator('#q').fill('C/Bikes');
  await page.locator('#q').press('Enter');
  await page.getByRole('heading', { name: 'c/bikes', exact: true }).waitFor();
  // Other registry requests (metadata, versions) can land after the search under load; what matters
  // is that the name search asked for the normalised prefix.
  assert.ok(lookups.some(url => url.searchParams.get('prefix') === 'bikes'), 'name search asks the registry for the normalised prefix');
  assert.match(await page.getByRole('link', { name: 'Review and join →', exact: true }).getAttribute('href'), new RegExp(`c=${community}&name=bikes`));
  await page.getByRole('link', { name: 'Browse registry →', exact: true }).click();
  await page.getByRole('heading', { name: 'Community registry', exact: true }).waitFor();
  await page.getByRole('heading', { name: 'c/bikes', exact: true }).waitFor();
  if (shots) await page.screenshot({ path: join(shots, 'registry-desktop.png'), fullPage: true });

  unavailable = true;
  await page.getByRole('searchbox', { name: 'Community name' }).fill('offline');
  await page.getByRole('button', { name: 'Search registry', exact: true }).click();
  await page.getByRole('button', { name: 'Retry registry search', exact: true }).waitFor();
  assert.equal(await page.getByRole('heading', { name: 'c/bikes', exact: true }).count(), 0, 'Old results must clear after a failed new lookup');
  unavailable = false;
  await page.getByRole('button', { name: 'Retry registry search', exact: true }).click();
  await page.getByRole('heading', { name: 'No public names starting with “offline”.', exact: true }).waitFor();

  await page.getByRole('searchbox', { name: 'Community name' }).fill('');
  await page.getByRole('button', { name: 'Search registry', exact: true }).click();
  await page.getByRole('heading', { name: 'c/bikes', exact: true }).waitFor();
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    await page.getByRole('navigation', { name: 'Footer', exact: true }).getByRole('link', { name: 'Public registry', exact: true }).click();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `No horizontal overflow at ${width}px`);
  }
  if (shots) {
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.screenshot({ path: join(shots, 'registry-mobile.png'), fullPage: true });
  }
  assert.deepEqual(errors, []);
  console.log('Discovery UI passed: local search privacy, signed community lookup, join link, registry navigation, retry, footer and 320/390px layouts.');
} finally {
  await browser?.close();
  await server.close();
}
