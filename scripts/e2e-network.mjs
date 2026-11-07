// End to end, with nothing run by the project: two browsers (two people, two keys, two
// IndexedDBs) talk through a hoi-node home node over WebSocket.
//
//   mira: stirs the swarm and makes a key → starts a community → pairs with the home node → posts
//   lena: opens mira's invite link → makes a key (skipping the stir) → joins → pairs → sees the post → replies
//   mira: sees the reply arrive live
//
// Needs: `pnpm core:wasm && pnpm build:web` and `cargo build -p hoi-node` (in core/).
// Usage: node scripts/e2e-network.mjs [--shots <dir>] [--headed]
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { KEY_WORK_TIMEOUT, STAMP_TIMEOUT, launch } from './e2e-browser.mjs';
import { unlockBrowser } from './browser-vault.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const shots = args.includes('--shots') ? resolve(args[args.indexOf('--shots') + 1]) : null;
if (shots) mkdirSync(shots, { recursive: true });
const NODE_PORT = 17842;
const WEB_PORT = 4180;
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
    const timeout = setTimeout(() => finish(new Error(`${cmd} not ready:\n${out}`)), 30_000);
    const finish = (error, match) => {
      clearTimeout(timeout);
      if (error) rej(error);
      else res({ child: c, match });
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

// ------------------------------------------------------------------ the home node and the app
const data = mkdtempSync(join(tmpdir(), 'hoi-node-'));
const { match: pairing } = await start(
  join(root, 'core/target/debug', process.platform === 'win32' ? 'hoi-node.exe' : 'hoi-node'),
  ['--data', data, '--listen', `127.0.0.1:${NODE_PORT}`, '--name', 'the pi', '--show-pairing', '--no-port-mapping'],
  // The page at ${WEB_PORT} may open the loopback listener (other origins are refused).
  { env: { ...process.env, HOI_ALLOWED_ORIGINS: `http://localhost:${WEB_PORT}` } },
  (out) => out.match(/hoi:\/\/pair\?\S+/)?.[0],
).catch((e) => fail(e.message));
console.log(`✓ hoi-node up, pairing link ${pairing.slice(0, 48)}…`);

await start(process.execPath, [join(root, 'apps/web/node_modules/vite/bin/vite.js'), 'preview', '--port', String(WEB_PORT), '--strictPort'], { cwd: join(root, 'apps/web') }, (out) => out.includes(String(WEB_PORT))).catch((e) => fail(e.message));
const base = `http://localhost:${WEB_PORT}/`;
console.log(`✓ web app at ${base}`);

const browser = await launch({ headless: !args.includes('--headed') });
const errors = [];
async function person(name) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(`${name}: ${e}`));
  page.on('console', (m) => m.type() === 'error' && errors.push(`${name}: ${m.text()}`));
  page.setDefaultTimeout(30_000);
  return page;
}
const snap = async (page, name) => shots && page.screenshot({ path: join(shots, `${name}.png`) });

/** "Say hoi": stir the swarm (or skip), watch the real identity work try by try, meet the key, go on. */
async function sayHoi(page, who, { stir, next }) {
  const act1 = page.getByRole('heading', { name: 'Stir the swarm.' });
  await act1.waitFor();
  if (stir) {
    const box = await page.locator('.swarm').boundingBox();
    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;
    const r = Math.min(box.height * 0.4, box.width * 0.42);
    // back and forth through the swarm until the ring is full and it moves on by itself
    for (let row = 0; row < 80 && (await act1.count()); row++) {
      const y = cy - r + 2 * r * ((row * 0.618) % 1);
      await page.mouse.move(cx - r, y);
      await page.mouse.move(cx + r, y + 12, { steps: 12 });
    }
  } else {
    await page.getByRole('button', { name: 'Skip', exact: true }).click();
  }
  // the miners report every Argon2id try: the strip shows real hash bits
  await page.locator('.strip .bits').waitFor();
  await page.getByRole('button', { name: 'Meet your key →' }).click({ timeout: KEY_WORK_TIMEOUT });
  const tries = await page.locator('.strip .tally .num').first().textContent();
  await page.getByRole('heading', { name: new RegExp(`Meet your key, ${who}`) }).waitFor();
  await page.getByRole('button', { name: next }).click();
  console.log(`✓ ${who} ${stir ? 'stirred' : 'skipped stirring'} and made a key (${tries} tries)`);
}

async function pair(page, who, link = pairing) {
  await page.goto(`${base}#/you`);
  await unlockBrowser(page);
  await page.fill('#y-pair', link);
  await page.getByRole('button', { name: 'Pair', exact: true }).click();
  await page.getByText(/via the pi/).first().waitFor();
  console.log(`✓ ${who} paired: "via the pi"`);
}

// ------------------------------------------------------------------ mira
const mira = await person('mira');
await mira.goto(base);
await unlockBrowser(mira);
await mira.fill('input.input.big', 'mira');
await mira.getByRole('button', { name: /Start one/ }).click();
await sayHoi(mira, 'mira', { stir: true, next: 'Start my community →' });
await mira.waitForURL(/#\/new/);
await mira.fill('#n-name', 'privacy');
await mira.fill('#n-desc', 'Practical privacy for normal people.');
await mira.fill('#n-rules', '1. Be kind.');
// This scenario uses invites and its own home node; public registration is covered separately.
const publicName = mira.getByRole('checkbox', { name: 'Reserve this name online' });
if (await publicName.isEnabled()) await publicName.uncheck();
await mira.getByRole('button', { name: /Make c\/privacy/ }).click();
const invite = await mira.getByLabel(/Web link/).inputValue();
if (!invite.includes('#/join?code=')) fail('no encoded invite link');
console.log('✓ mira started c/privacy and got an invite link');
await snap(mira, '01-mira-invite');
await mira.keyboard.press('Escape');

await pair(mira, 'mira');
await mira.goto(`${base}#/`);
await unlockBrowser(mira);
await mira.getByRole('button', { name: '+ Post' }).first().click();
await mira.fill('#ctitle', 'Which messengers survived the audit?');
await mira.fill('#ctext', 'Short version: three apps close the gap.');
await mira.getByRole('button', { name: 'Sign & send' }).click();
await mira.getByText('Which messengers survived the audit?').first().waitFor({ timeout: STAMP_TIMEOUT });
console.log('✓ mira posted (signed, stamped, sent)');
await snap(mira, '02-mira-posted');

// After posting, mira shortens the window. A device that joins later still takes the post,
// made under the limits of its day.
await mira.getByText('c/privacy').first().click();
await mira.waitForURL(/#\/c\//);
await mira.goto(`${mira.url().split('/manage')[0]}/manage`);
await unlockBrowser(mira);
await mira.locator('label', { hasText: '30 d' }).first().click();
await mira.getByRole('button', { name: 'Propose new limits' }).click();
await mira.locator('.kv', { hasText: 'Posts fade after' }).getByText('30 days').waitFor();
console.log('✓ mira shortened the window to 30 days after posting');
await mira.goto(`${base}#/`);
await unlockBrowser(mira);

// ------------------------------------------------------------------ lena
const lena = await person('lena');
await lena.goto(invite);
await unlockBrowser(lena);
await lena.fill('input.input.big', 'lena');
await lena.getByRole('button', { name: /Make my key & see c\/privacy/ }).click();
await sayHoi(lena, 'lena', { stir: false, next: 'See c/privacy →' });
await lena.getByRole('button', { name: /^Join c\/privacy/ }).click();
await lena.getByRole('dialog', { name: 'Turn on your VPN before joining' }).getByRole('button', { name: 'Continue joining' }).click();
await lena.waitForURL(/#\/c\//);
console.log('✓ lena opened the invite, made a key and joined');
// The node's first link paired mira and is spent: her browser shows its current one.
await mira.goto(`${base}#/you`);
await unlockBrowser(mira);
await mira.getByRole('button', { name: '+ Link a device' }).click();
const nextPair = await mira.getByLabel(/Pairing link for your other browser or phone/).inputValue();
await mira.keyboard.press('Escape');
await mira.goto(`${base}#/`);
await unlockBrowser(mira);
await pair(lena, 'lena', nextPair);
await lena.goto(`${base}#/`);
await unlockBrowser(lena);
await lena.getByText('Which messengers survived the audit?').first().waitFor();
console.log("✓ lena sees mira's post, synced through the home node");
await snap(lena, '03-lena-feed');

await lena.getByText('Which messengers survived the audit?').first().click();
await lena.waitForURL(/#\/p\//);
await lena.locator('textarea').first().fill('Defaults count, not capability.');
await lena.getByRole('button', { name: /Sign & send/ }).first().click();
await lena.getByText('Defaults count, not capability.').first().waitFor({ timeout: STAMP_TIMEOUT });
console.log('✓ lena replied');

await mira.getByText('Which messengers survived the audit?').first().click();
await mira.waitForURL(/#\/p\//);
await mira.getByText('Defaults count, not capability.').first().waitFor({ timeout: 30_000 });
console.log("✓ mira sees lena's reply, live");
await snap(mira, '04-mira-reply');

// ------------------------------------------------------------------ moderation travels too
const reply = mira.locator('div.c').filter({ hasText: 'Defaults count, not capability.' }).first();
await reply.getByRole('button', { name: 'Remove', exact: true }).click();
await reply.getByPlaceholder('rule 2, doxxing').fill('rule 1, be kind');
await reply.getByRole('button', { name: /Remove & sign/ }).click();
await mira.getByText(/removal pending: rule 1, be kind/).first().waitFor();
console.log('✓ mira removed the reply with a public reason; it naps while members can veto');
await lena.getByText(/removal pending: rule 1, be kind/).first().waitFor({ timeout: 30_000 });
console.log('✓ lena sees it napping with the reason, still peekable');
await snap(lena, '05-lena-pending');
// removals for private data can't wait: Poof at once, on both devices
await lena.locator('textarea').first().fill('Here is where jonas lives, for anyone curious.');
await lena.getByRole('button', { name: /Sign & send/ }).first().click();
const doxx = mira.locator('div.c').filter({ hasText: 'Here is where jonas lives' }).first();
await doxx.waitFor({ timeout: STAMP_TIMEOUT });
await doxx.getByRole('button', { name: 'Remove', exact: true }).click();
await doxx.getByPlaceholder('rule 2, doxxing').fill('privacy: an address');
await doxx.getByRole('button', { name: /Remove & sign/ }).click();
await mira.getByText(/Removed by mira · privacy: an address/).first().waitFor();
await lena.getByText(/Removed by mira · privacy: an address/).first().waitFor({ timeout: 30_000 });
console.log('✓ a removal for private data takes effect at once: "Poof." on both devices');
await snap(lena, '05-lena-poof');

// ------------------------------------------------------------------ pictures travel too
// lena's browser asks the home node for the picture while it is still coming up from mira's
const picture = Buffer.from(
  await mira.evaluate(async () => {
    const c = new OffscreenCanvas(480, 320);
    const g = c.getContext('2d');
    const grad = g.createLinearGradient(0, 0, 480, 320);
    grad.addColorStop(0, '#2f55e6');
    grad.addColorStop(1, '#ff6b4a');
    g.fillStyle = grad;
    g.fillRect(0, 0, 480, 320);
    g.fillStyle = '#ffe16a';
    g.beginPath();
    g.arc(170, 150, 90, 0, Math.PI * 2);
    g.fill();
    return Array.from(new Uint8Array(await (await c.convertToBlob({ type: 'image/png' })).arrayBuffer()));
  }),
);
await mira.goto(`${base}#/`);
await unlockBrowser(mira);
await mira.getByRole('button', { name: '+ Post' }).first().click();
await mira.fill('#ctitle', 'The relay under my desk');
await mira.locator('input[type=file]').setInputFiles({ name: 'relay.png', mimeType: 'image/png', buffer: picture });
await mira.getByRole('button', { name: 'Sign & send' }).click();
await mira.locator('article').filter({ hasText: 'The relay under my desk' }).locator('a.preview img').waitFor({ timeout: STAMP_TIMEOUT });
console.log('✓ mira posted a picture; her feed shows it');
await lena.goto(`${base}#/`);
await unlockBrowser(lena);
const pictureRow = lena.locator('article').filter({ hasText: 'The relay under my desk' }).first();
await pictureRow.locator('a.preview img').waitFor({ timeout: 30_000 });
if (!(await pictureRow.locator('a.preview img').evaluate((i) => i.complete && i.naturalWidth === 480))) fail('the preview in lena\'s feed did not open');
console.log("✓ lena sees mira's picture in her feed, fetched through the home node");
await snap(lena, '06-lena-picture');
await pictureRow.getByRole('link', { name: 'The relay under my desk' }).click();
await lena.waitForURL(/#\/p\//);
await lena.locator('.media img').first().waitFor();
console.log('✓ and full size on the post');
await lena.goto(`${base}#/`);
await unlockBrowser(lena);
await browser.close();
if (errors.length) fail(`console errors:\n  ${errors.join('\n  ')}`);
console.log('✓ no console errors · project servers involved: none');
cleanup();
process.exit(0);
