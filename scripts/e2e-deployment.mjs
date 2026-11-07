// Static deployment boundary checks against the assembled dist/, served like a plain static host
// (scripts/serve-static.mjs) unless HOI_DEPLOYMENT_TEST_URL names a local one. Independent
// registry and relay behavior is exercised by e2e-registry-browser.mjs and e2e-private-relay.mjs.
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { serveStatic } from './serve-static.mjs';
import { consentAccepted } from './browser-vault.mjs';

const local = process.env.HOI_DEPLOYMENT_TEST_URL ? null : await serveStatic({ root: fileURLToPath(new URL('../dist', import.meta.url)) });
const target = new URL(process.env.HOI_DEPLOYMENT_TEST_URL || local.origin);
assert.ok(['127.0.0.1', '[::1]', 'localhost'].includes(target.hostname));
assert.ok(!target.username && !target.password && target.pathname === '/' && !target.search && !target.hash);
const origin = target.origin;
let ready = false;
for (let i = 0; i < 120; i++) {
  try { ready = (await fetch(origin, { signal: AbortSignal.timeout(2000) })).ok; } catch { /* starting */ }
  if (ready) break;
  await new Promise(resolve => setTimeout(resolve, 500));
}
assert.ok(ready, 'Local static deployment must start within 60 seconds');
for (const path of ['/', '/protocol/', '/app/', '/release.json', '/app/release.json']) {
  const response = await fetch(origin + path);
  assert.equal(response.status, 200, path);
  assert.equal(response.headers.get('set-cookie'), null);
}
// A plain static host sends no security headers, so every page carries its own policy in a <meta>.
// No page may need inline styles or scripts.
for (const path of ['/', '/protocol/', '/relay/', '/app/']) {
  const html = await (await fetch(origin + path)).text();
  const csp = html.match(/<meta http-equiv="Content-Security-Policy" content="([^"]+)">/)?.[1] ?? '';
  const directive = (name) => csp.split(';').map(d => d.trim()).find(d => d.startsWith(name + ' ')) ?? '';
  assert.match(directive('default-src'), /^default-src 'self'$/, `${path} default-src`);
  assert.equal(directive('style-src'), "style-src 'self'", `${path} style-src allows nothing inline`);
  assert.ok(!/'unsafe-inline'|'unsafe-eval'/.test(csp), `${path} CSP has no unsafe-inline or unsafe-eval`);
  assert.match(directive('object-src'), /'none'/, `${path} object-src`);
  assert.match(directive('base-uri'), /'none'/, `${path} base-uri`);
  assert.match(html, /<meta name="referrer" content="no-referrer">/, `${path} referrer policy`);
}
// Search engines may read and list the website, preview or not; a preview app keeps itself out.
assert.doesNotMatch(await (await fetch(origin + '/robots.txt')).text(), /^Disallow: \/\s*$/m, 'robots.txt lets crawlers read the site');
const sitemap = await (await fetch(origin + '/sitemap.xml')).text();
for (const path of ['/', '/download/', '/verify/', '/relay/', '/communities/', '/protocol/']) {
  const html = await (await fetch(origin + path)).text();
  assert.match(html, /<meta name="description" content="[^"]{50,160}">/, `${path} has a description search results can show whole`);
  const canonical = html.match(/<link rel="canonical" href="([^"]+)">/)?.[1] ?? '';
  assert.equal(URL.canParse(canonical) && new URL(canonical).pathname, path, `${path} names itself as canonical`);
  assert.ok(sitemap.includes(`<loc>${canonical}</loc>`), `${path} is in sitemap.xml`);
  assert.doesNotMatch(html, /<meta name="robots"/, `${path} is indexable`);
}
const preview = (await (await fetch(origin + '/release.json')).json()).channel === 'preview';
assert.equal(/<meta name="robots" content="noindex/.test(await (await fetch(origin + '/app/')).text()), preview, 'a preview app stays out of search results');
for (const path of ['/registry/', '/registry/v1/meta', '/registry/v2/communities?prefix=', '/relay/v1/meta']) {
  const response = await fetch(origin + path, { redirect: 'manual' });
  assert.equal(response.status, 404, `${path} must not expose a service on the static host`);
}
for (const path of ['/registry/v2/communities', '/relay/v1/objects']) {
  const response = await fetch(origin + path, { method: 'PUT', body: '{}' });
  assert.ok([404, 405].includes(response.status), `${path} cannot accept user data`);
}
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
try {
  const context = await browser.newContext({ reducedMotion: 'reduce' });
  const page = await context.newPage();
  const errors = [], requests = [], violations = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => requests.push(request));
  await page.exposeFunction('cspViolation', (v) => violations.push(v));
  await context.addInitScript(() => document.addEventListener('securitypolicyviolation', e => window.cspViolation(`${e.violatedDirective} ${e.blockedURI || 'inline'} at ${location.pathname}`)));
  await page.goto(origin, { waitUntil: 'networkidle' });
  await page.locator('#draft').fill('This demo text must stay in this browser');
  await page.locator('#signBtn').click();
  await page.waitForTimeout(1500);
  assert.equal((await context.cookies()).length, 0);
  assert.deepEqual(await page.evaluate(async () => ({ local: localStorage.length, session: sessionStorage.length, databases: (await indexedDB.databases()).length })), { local: 0, session: 0, databases: 0 });
  assert.ok(requests.every(request => new URL(request.url()).origin === origin && request.method() === 'GET'));
  assert.equal(requests.filter(request => ['fetch', 'xhr', 'websocket'].includes(request.resourceType())).length, 0, 'marketing page makes no background requests');
  await page.goto(origin + '/protocol/');
  await page.getByRole('heading', { name: 'What runs where, and what it handles.' }).waitFor();
  // The site's own pages (its "Run a relay" page lives at /relay/) are visited on purpose.
  const visited = new Set();
  for (const path of ['/communities/', '/relay/', '/verify/', '/download/', '/no-such-page/']) {
    visited.add(origin + path);
    await page.goto(origin + path, { waitUntil: 'networkidle' });
  }
  // App consent belongs to the app fixture; marketing pages must start with empty storage.
  await consentAccepted(context);
  await page.goto(origin + '/app/', { waitUntil: 'networkidle' });
  const backend = requests.filter(request => !visited.has(request.url()) && /\/(registry|relay)(\/|$)/.test(new URL(request.url()).pathname));
  assert.equal(backend.length, 0, 'default static client has no implicit backend');
  assert.deepEqual(errors, []);
  assert.deepEqual(violations, [], 'no page breaks its own Content-Security-Policy');
  console.log('PASS: static files, page security policies, no CSP violations, no registry/relay API, local-only marketing demos, no cookies/persistence/telemetry, browser client loads.');
} finally {
  await browser.close();
  await local?.close();
}
