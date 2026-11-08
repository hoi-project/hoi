// End to end over WebRTC, with nothing run by the project: a hoi-node, a computer's browser that
// holds the key, a phone that pairs by "scanning" and takes the key with four digits, and a
// member's browser that joins from an invite and catches up from the node while nobody who
// invited it is around.
//
//   mira (desktop browser): makes a key → starts a community → pairs with the node → posts
//   phone (mobile browser): opens the pairing QR's link → Connect (WebRTC, the loopback address is
//                           skipped on phones) → Link this phone → four digits
//   mira: "Link a new device?" with the same digits → yes
//   phone: signs as mira, sees the post, replies; its traffic view says WebRTC, DTLS
//   kim (desktop browser, not paired): opens mira's invite → makes a key → joins → catches up from
//                                      the node over WebRTC ("via a member's computer")
//
// Needs: `pnpm core:wasm && pnpm build:web` and `cargo build -p hoi-node` (in core/).
// Usage: node scripts/e2e-rtc.mjs [--shots <dir>] [--headed]   (CHROMIUM_PATH picks a browser)
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { devices } from 'playwright';
import { KEY_WORK_TIMEOUT, STAMP_TIMEOUT, device, launch } from './e2e-browser.mjs';
import { consentAccepted, unlockBrowser } from './browser-vault.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const shots = args.includes('--shots') ? resolve(args[args.indexOf('--shots') + 1]) : null;
if (shots) mkdirSync(shots, { recursive: true });
const NODE_PORT = 17852;
const RTC_PORT = 17854;
const WEB_PORT = 4181;
const origin = `http://127.0.0.1:${WEB_PORT}`;
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
// A step that times out: say what the pages logged before going.
process.on('unhandledRejection', (e) => {
  console.error(String(e?.message ?? e).split('\n')[0]);
  if (errors.length) console.error(`console errors so far:\n  ${errors.join('\n  ')}`);
  cleanup();
  process.exit(1);
});

function start(cmd, argv, opts, ready) {
  return new Promise((res, rej) => {
    const c = spawn(cmd, argv, { ...opts, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    children.push(c);
    let out = '';
    const timeout = setTimeout(() => finish(new Error(`${cmd} not ready:\n${out}`)), 30_000);
    const finish = (error, match) => {
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

// ------------------------------------------------------------------ the node and the app
const data = mkdtempSync(join(tmpdir(), 'hoi-node-'));
const node = await start(
  join(root, 'core/target/debug', process.platform === 'win32' ? 'hoi-node.exe' : 'hoi-node'),
  ['--data', data, '--listen', `127.0.0.1:${NODE_PORT}`, '--name', 'the pi', '--rtc-port', String(RTC_PORT),
    '--rtc-advertise', `127.0.0.1:${RTC_PORT}`, '--show-pairing', '--no-port-mapping'],
  // The page at ${WEB_PORT} may open the loopback listener (other origins are refused).
  { env: { ...process.env, HOI_ALLOWED_ORIGINS: origin } },
  (out) => (/relaying for members/.test(out) ? out : null),
).catch((e) => fail(e.message));
const pairing = node.match.match(/hoi:\/\/pair\?\S+/)?.[0];
if (!pairing) fail('the node did not print a pairing link');
if (!node.match.includes(`WebRTC on UDP ${RTC_PORT} for browsers and phones:`)) fail('the node did not advertise a WebRTC route');
console.log(`✓ hoi-node up, WebRTC on UDP ${RTC_PORT}`);

await start(process.execPath, [join(root, 'apps/web/node_modules/vite/bin/vite.js'), 'preview', '--host', '127.0.0.1', '--port', String(WEB_PORT), '--strictPort'], { cwd: join(root, 'apps/web') }, (out) => out.includes(String(WEB_PORT))).catch((e) => fail(e.message));
const base = `${origin}/`;
console.log(`✓ web app at ${base}`);

const browser = await launch({ headless: !args.includes('--headed') });
const errors = [];
async function person(name, device) {
  const ctx = await browser.newContext({ ...(device ?? { viewport: { width: 1280, height: 900 } }), serviceWorkers: 'block' });
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
  console.log(`✓ ${who} made a key`);
}

// ------------------------------------------------------------------ mira, at her computer
const mira = await person('mira');
await mira.goto(base);
await unlockBrowser(mira);
await mira.fill('input.input.big', 'mira');
await mira.getByRole('button', { name: /Start one/ }).click();
await sayHoi(mira, 'mira', 'Start my community →');
await mira.waitForURL(/#\/new/);
await mira.fill('#n-name', 'privacy');
await mira.fill('#n-desc', 'Practical privacy for normal people.');
await mira.fill('#n-rules', '1. Be kind.');
// This scenario uses invites and its own home node; public registration is covered separately.
const publicName = mira.getByRole('checkbox', { name: 'Reserve this name online' });
if (await publicName.isEnabled()) await publicName.uncheck();
await mira.getByRole('button', { name: /Make c\/privacy/ }).click();
await mira.getByLabel(/Web link/).waitFor();
await mira.keyboard.press('Escape');
await mira.goto(`${base}#/you`);
await unlockBrowser(mira);
await mira.fill('#y-pair', pairing);
await mira.getByRole('button', { name: 'Pair', exact: true }).click();
await mira.getByText(/via the pi/).first().waitFor();
console.log('✓ mira started c/privacy and paired her browser with the node (loopback WebSocket)');
await mira.goto(`${base}#/`);
await unlockBrowser(mira);
await mira.getByRole('button', { name: '+ Post' }).first().click();
await mira.fill('#ctitle', 'Phones welcome now');
await mira.fill('#ctext', 'One scan, four digits, done.');
await mira.getByRole('button', { name: 'Sign & send' }).click();
await mira.getByText('Phones welcome now').first().waitFor({ timeout: STAMP_TIMEOUT });
console.log('✓ mira posted');

// ------------------------------------------------------------------ her phone scans the QR
// The node's first link paired mira's browser and is spent: her browser shows the next one.
await mira.goto(`${base}#/you`);
await unlockBrowser(mira);
await mira.getByRole('button', { name: '+ Link a device' }).click();
const nextPair = await mira.getByLabel(/Pairing link for your other browser or phone/).inputValue();
await mira.keyboard.press('Escape');
// Keep the node's opaque pairing code and open it against the app built for this test.
const pairRoute = new URL(nextPair).hash;
if (!pairRoute.startsWith('#/pair?code=')) fail('the browser did not show an encoded pairing link');
const phone = await person('phone', device(devices['Pixel 7']));
await phone.goto(`${base}${pairRoute}`);
await unlockBrowser(phone);
await phone.getByRole('heading', { name: 'Connect to the pi.' }).waitFor();
await snap(phone, '01-phone-connect');
// "Connect" for a node on this device or network; a machine whose only address is a public one
// (or, as in some containers, a documentation range) asks to confirm it by address
await phone.getByRole('button', { name: /^(Connect|Yes, connect to .+)$/ }).click();
await phone.getByText(/over WebRTC, encrypted end to end/).waitFor({ timeout: 30_000 });
console.log('✓ the phone connected to the node over WebRTC (no certificate, no server in between)');
await phone.getByRole('button', { name: /Link this browser|Link this phone/ }).click();
await phone.locator('.digits').first().waitFor();
const digits = (await phone.locator('.digits').textContent()).replace(/\s/g, '');
if (!/^\d{4}$/.test(digits)) fail(`no check digits on the phone: ${digits}`);
await snap(phone, '02-phone-digits');
console.log(`✓ the phone asks to be linked and shows ${digits}`);

await mira.getByRole('heading', { name: 'Link a new device?' }).waitFor({ timeout: 30_000 });
const shown = (await mira.locator('dialog .digits').textContent()).replace(/\s/g, '');
if (shown !== digits) fail(`mira sees ${shown}, the phone shows ${digits}`);
await snap(mira, '03-mira-ask');
await mira.getByRole('button', { name: 'Yes, link it' }).click();
console.log('✓ mira sees the same four digits on her computer and says yes');

await phone.getByText(/You're set/).waitFor({ timeout: 30_000 });
await phone.getByRole('button', { name: 'Open hoi' }).click();
await phone.getByText('Phones welcome now').first().waitFor({ timeout: 30_000 });
console.log('✓ the phone signs as mira and caught up on c/privacy through the node');
await snap(phone, '04-phone-feed');
await phone.getByText('Phones welcome now').first().click();
await phone.waitForURL(/#\/p\//);
await phone.locator('textarea').first().fill('Hello from the phone.');
await phone.getByRole('button', { name: /Sign & send/ }).first().click();
await mira.goto(`${base}#/`);
await unlockBrowser(mira);
await mira.getByText('Phones welcome now').first().click();
await mira.getByText('Hello from the phone.').first().waitFor({ timeout: STAMP_TIMEOUT });
console.log("✓ the phone's reply arrived on mira's computer, live");
// Her home node has delivered records now (the phone's reply), so her browser takes its routes
// and the invite it makes carries them, WebRTC included.
await mira.goto(`${base}#/communities`);
await unlockBrowser(mira);
await mira.getByText('c/privacy').first().click();
await mira.waitForURL(/#\/c\//);
await mira.getByRole('button', { name: /Invite/ }).first().click();
const invite = await mira.getByLabel(/Web link/).inputValue();
// Invite codes mask their routes; the dialog lists the addresses they carry.
const listed = mira.locator('details.addresses');
if (!(await listed.count())) fail(`the invite lists no addresses: ${(await mira.locator('body').innerText()).slice(0, 1200)}`);
await listed.locator('summary').click();
const carried = await listed.textContent();
await mira.keyboard.press('Escape');
if (!carried.includes(`:${RTC_PORT}`)) fail(`the invite carries no WebRTC route to the node: ${carried.slice(0, 200)}…`);
console.log('✓ her invite points at the node over WebRTC, so others get in while she is away');

await phone.goto(`${base}#/you`);
await unlockBrowser(phone);
await phone.getByText('encrypted · WebRTC, DTLS').first().waitFor({ timeout: 15_000 });
await snap(phone, '05-phone-traffic');
console.log('✓ the phone shows what passes, and how: encrypted · WebRTC, DTLS');

// ------------------------------------------------------------------ kim joins while mira is away
await mira.close();
const kim = await person('kim');
await kim.goto(invite);
await unlockBrowser(kim);
await kim.fill('input.input.big', 'kim');
await kim.getByRole('button', { name: /Make my key & see c\/privacy/ }).click();
await sayHoi(kim, 'kim', 'See c/privacy →');
await kim.getByRole('button', { name: /^Join c\/privacy/ }).click();
await kim.getByRole('dialog', { name: 'Turn on your VPN before joining' }).getByRole('button', { name: 'Continue joining' }).click();
await kim.waitForURL(/#\/c\//);
await kim.getByText('Phones welcome now').first().waitFor({ timeout: 60_000 });
console.log("✓ kim joined with mira's invite and caught up from the node over WebRTC, mira's browser closed");
await kim.goto(`${base}#/you`);
await unlockBrowser(kim);
await kim.getByText(/via a member's computer/).first().waitFor({ timeout: 15_000 });
await snap(kim, '06-kim-member');

await browser.close();
const benign = (e) => /favicon|ERR_CONNECTION_REFUSED|WebSocket connection to 'ws:\/\/127\.0\.0\.1/.test(e);
const real = errors.filter((e) => !benign(e));
if (real.length) fail(`console errors:\n  ${real.join('\n  ')}`);
console.log('✓ no console errors · project servers involved: none');
cleanup();
process.exit(0);
