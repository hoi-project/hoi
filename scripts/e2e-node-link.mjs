// End to end with nothing configured anywhere: the app built with no registry and no relay, and
// a home node started with no registry, no relay and no node named in them.
//
//   lena's pi (hoi-node): a home node, so a public node; prints its node link
//   mira (browser, no home node): opens lena's node link → starts c/gardening, which her browser
//                                 asks lena's pi to carry → posts, after being told once that
//                                 nobody can read it until a home node carrying it has it
//   kim (fresh browser, no home node): opens the same link → c/gardening is listed under
//                                      Communities → joins → reads mira's post with mira's
//                                      browser closed
//
// Node lists between nodes and on to browsers are covered by hoi-core's tests (tests/nodes.rs);
// a browser run across two nodes is still to be added here.
//
// Needs: `pnpm build` (or `pnpm core:wasm && pnpm build:web`) with VITE_HOI_REGISTRY_URL and
// VITE_HOI_RELAY_URLS unset, and `cargo build -p hoi-node` (in core/).
// Usage: node scripts/e2e-node-link.mjs [--shots <dir>] [--headed]   (CHROMIUM_PATH picks a browser)
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { KEY_WORK_TIMEOUT, STAMP_TIMEOUT, launch } from './e2e-browser.mjs';
import { consentAccepted, unlockBrowser } from './browser-vault.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const shots = args.includes('--shots') ? resolve(args[args.indexOf('--shots') + 1]) : null;
if (shots) mkdirSync(shots, { recursive: true });
const WEB_PORT = 4191;
const children = [];
const fail = (msg) => {
  console.error(`✗ ${msg}`);
  cleanup();
  process.exit(1);
};
function cleanup() {
  for (const c of children) c.kill('SIGTERM');
}
process.on('exit', cleanup);

function start(cmd, argv, opts, ready) {
  return new Promise((res, rej) => {
    const c = spawn(cmd, argv, { ...opts, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    children.push(c);
    let out = '';
    const timeout = setTimeout(() => finish(new Error(`${cmd} not ready:\n${out}`)), 60_000);
    let done = false;
    const finish = (error, match) => {
      if (done) return;
      done = true;
      clearTimeout(timeout);
      if (error) rej(error);
      else res({ child: c, match, out: () => out });
    };
    const on = (d) => {
      out += d;
      const m = ready(out);
      if (m) finish(null, m);
    };
    c.stdout.on('data', on);
    c.stderr.on('data', on);
    c.once('error', finish);
    c.once('exit', (code) => finish(new Error(`${cmd} exited ${code}:\n${out}`)));
  });
}

// Nothing is configured into the app: the build this test serves must not name a service.
const connect = readFileSync(join(root, 'apps/web/dist/index.html'), 'utf8').match(/connect-src([^;"]*)/)?.[1].trim();
if (connect !== "'self' ws: wss: https:") fail(`the app build lets the page reach "${connect}": build it with nothing configured`);

// ------------------------------------------------------------------ a home node
const hoiNode = join(root, 'core/target/debug', process.platform === 'win32' ? 'hoi-node.exe' : 'hoi-node');
/** The node links a hoi-node printed, each with what it holds (the line under it): the addresses
 * themselves are packed into the link's code. */
const linksIn = (out) => [...out.matchAll(/https:\/\/\S+#\/node\?(\S+)\s+it holds (\d+) WebRTC routes? and (\d+) native routes?(, and the operator's notice, signed)?/g)]
  .map((m) => ({ query: m[1], rtc: Number(m[2]), native: Number(m[3]), notice: !!m[4] }));
/** The last node link a node printed that names both a WebRTC and a native route. */
const linkIn = (out) => linksIn(out).filter((l) => l.rtc && l.native).pop();
// Lena's notice: who runs the node, written the way the installer writes it.
const lenaData = mkdtempSync(join(tmpdir(), 'hoi-lena-'));
const noticeFile = join(lenaData, 'notice.json');
writeFileSync(noticeFile, JSON.stringify({
  v: 1, node: "lena's pi", operator: 'Lena', contact: 'lena at example dot org', country: 'XX', provider: 'a shelf in the hallway',
  processes: 'Nothing beyond that: the node keeps the open communities it carries and the addresses of the other public nodes it reached.',
  ipLogging: false, retentionDays: 0, updated: '2026-11-03',
}));
const lenas = await start(
  hoiNode,
  ['--data', lenaData, '--listen', '127.0.0.1:17862', '--rtc-port', '17864',
    '--rtc-advertise', '127.0.0.1:17864', '--name', "lena's pi", '--no-port-mapping', '--notice', noticeFile],
  {},
  linkIn,
).catch((e) => fail(e.message));
const lenaLink = lenas.match.query;
if (!/a public node: passing on the public nodes/.test(lenas.out())) fail(`a plain hoi-node is not a public node:\n${lenas.out()}`);
if (!/operator notice: by Lena · contact lena at example dot org · server in XX/.test(lenas.out())) fail(`the node did not take the notice:\n${lenas.out()}`);
if (!lenas.match.notice) fail(`the node link carries no signed notice: ${lenaLink}`);
if (!lenaLink.startsWith('code=') || /127\.0\.0\.1|@/.test(lenaLink)) fail(`the node link shows its addresses: ${lenaLink}`);
if (!readFileSync(join(lenaData, 'notice-hash'), 'utf8').trim().match(/^[0-9a-f]{64}$/)) fail('no notice-hash file');
console.log("✓ lena's pi is up as a public node with no registry, relay or node configured, printed its node link with her signed notice in it");
await start(process.execPath, [join(root, 'apps/web/node_modules/vite/bin/vite.js'), 'preview', '--host', '127.0.0.1', '--port', String(WEB_PORT), '--strictPort'], { cwd: join(root, 'apps/web') }, (out) => out.includes(String(WEB_PORT))).catch((e) => fail(e.message));
const base = `http://127.0.0.1:${WEB_PORT}/`;
const nodeLink = `${base}#/node?${lenaLink}`;
console.log(`✓ web app at ${base}, nothing configured in it`);

const browser = await launch({ headless: !args.includes('--headed') });
const errors = [];
async function person(name) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' });
  await consentAccepted(ctx);
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(`${name}: ${e}`));
  page.on('console', (m) => m.type() === 'error' && errors.push(`${name}: ${m.text()}`));
  page.setDefaultTimeout(30_000);
  return page;
}
const snap = async (page, name) => shots && page.screenshot({ path: join(shots, `${name}.png`), fullPage: true });

/** "Say hoi" without stirring: watch the identity work, meet the key, go on. */
async function sayHoi(page, who, next) {
  await page.getByRole('heading', { name: 'Stir the swarm.' }).waitFor();
  await page.getByRole('button', { name: 'Skip', exact: true }).click();
  await page.getByRole('button', { name: 'Meet your key →' }).click({ timeout: KEY_WORK_TIMEOUT });
  await page.getByRole('heading', { name: new RegExp(`Meet your key, ${who}`) }).waitFor();
  await page.getByRole('button', { name: next }).click();
}

/** Opens lena's node link with no key yet: make one, then connect to the node it names. */
async function openNodeLink(page, who) {
  await page.goto(nodeLink);
  await unlockBrowser(page);
  await page.fill('input.input.big', who);
  await page.getByRole('button', { name: 'Make my key & look at this node →' }).click();
  await sayHoi(page, who, 'Look at this node →');
  await page.getByRole('heading', { name: 'A public node.' }).waitFor();
  // Lena's notice, read from the link and checked against her node's key, before any dial.
  await page.getByText("Join notice – lena's pi").waitFor();
  await page.getByText('This node is operated by Lena. Contact: lena at example dot org.').waitFor();
  await page.getByText('IP addresses are not logged.').waitFor();
  await page.getByRole('button', { name: 'Accept and connect' }).click();
  await page.waitForURL(/#\/communities/);
  await page.getByRole('status').filter({ hasText: /1 public node reached before, 1 connected now/ }).waitFor({ timeout: 60_000 });
}

// ------------------------------------------------------------------ mira starts a community
const mira = await person('mira');
await openNodeLink(mira, 'mira');
console.log("✓ mira opened lena's node link in a browser with nothing configured and reached the node");
await mira.getByRole('link', { name: '+ Start a community' }).click();
await mira.fill('#n-name', 'gardening');
await mira.fill('#n-desc', 'Seeds, soil and what grows on a balcony.');
await mira.getByRole('button', { name: 'Make c/gardening' }).click();
await mira.getByLabel(/Web link/).waitFor();
await mira.keyboard.press('Escape');
// Her browser asks the node it reached to carry what she founded; posting opens once it does.
await mira.getByRole('button', { name: '+ Post' }).first().click();
await mira.fill('#ctitle', 'Tomatoes on a north balcony?');
await mira.fill('#ctext', 'Mine get three hours of sun. Worth it?');
const send = mira.getByRole('button', { name: 'Sign & send' });
await mira.waitForFunction(() => !document.querySelector('form[aria-label="New post"] button[type="submit"]')?.disabled, null, { timeout: STAMP_TIMEOUT });
console.log("✓ lena's pi took c/gardening from mira's browser, unasked by anyone: posting is open");
await send.click();
const notice = mira.getByRole('dialog', { name: 'Nobody can read posts from this browser' });
await notice.getByText(/reaches others only through a home node that carries this community \(1 node connected now\)/).waitFor();
await snap(mira, '01-posting-notice');
await notice.getByRole('button', { name: 'Post through them' }).click();
await mira.locator('.toast').filter({ hasText: /Sent to the home nodes carrying this community/ }).waitFor();
console.log('✓ mira was told once that nobody can read her post until a home node has it, and posted through the node');
await mira.waitForTimeout(3000);
await mira.close();

// ------------------------------------------------------------------ kim finds it, with mira gone
const kim = await person('kim');
await openNodeLink(kim, 'kim');
const listed = kim.locator('a.crow', { hasText: 'c/gardening' });
await listed.waitFor({ timeout: 30_000 });
await snap(kim, '02-kim-communities');
console.log("✓ kim opened the same link and c/gardening is listed from lena's pi");
await listed.click();
await kim.getByRole('button', { name: 'Join c/gardening' }).click();
await kim.getByRole('dialog', { name: 'Turn on your VPN before joining' }).getByRole('button', { name: 'Continue joining' }).click();
await kim.waitForURL(/#\/c\//);
await kim.getByText('Tomatoes on a north balcony?').first().waitFor({ timeout: 60_000 });
console.log("✓ kim joined and reads mira's post from lena's pi, mira's browser closed");

await browser.close();
const benign = (e) => /favicon|ERR_CONNECTION_REFUSED|WebSocket connection to 'ws:\/\/127\.0\.0\.1/.test(e);
const real = errors.filter((e) => !benign(e));
if (real.length) fail(`console errors:\n  ${real.join('\n  ')}`);
console.log('✓ no console errors · registries, relays or nodes named by the project: none');
cleanup();
process.exit(0);
