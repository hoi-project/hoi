// Private communities with no relay anywhere, end to end over real links (SCATTER.md §5.13):
// real hoi-node holders keep the mailboxes, real browsers (the WASM core, IndexedDB) pick them,
// write into them and read from them.
//
//   two hoi-node holders (--hold-pieces): one serves WebRTC on this machine's loopback address,
//     the other on its network address, so a browser counts two places, as it would two
//     nodes in two networks. Neither is named anywhere in the app.
//   mira (browser): opens both nodes' links, starts a private community; her browser links to
//     both and makes them the group's mailboxes by itself, with no relay configured
//   kim (browser, no node link): asks to join; mira's invitation carries the mailboxes; kim's
//     browser links to them by itself, reads mira's post, and mira reads kim's reply
//   the holders: keep only sealed objects (`hoi-node admin pieces` counts them) and say so in
//     their notices
//
// Needs: `pnpm core:wasm` and `cargo build -p hoi-node` (in core/), and a non-link-local IPv4
// address on this machine besides loopback. Vite serves the app with no registry and no relay.
// Usage: node scripts/e2e-mailboxes.mjs   (CHROMIUM_PATH picks a browser)
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { createSocket } from 'node:dgram';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { networkInterfaces, tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { launch } from './e2e-browser.mjs';
import { createServer as createViteServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { consentAccepted } from './browser-vault.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const hoiNode = join(root, 'core/target/debug', process.platform === 'win32' ? 'hoi-node.exe' : 'hoi-node');
const children = [];
let vite;
let browser;
function cleanup() {
  for (const c of children) c.kill('SIGTERM');
}
process.on('exit', cleanup);
const fail = (msg) => {
  console.error(`✗ ${msg}`);
  cleanup();
  process.exit(1);
};

// Two places on one machine: loopback and a network address the browser can reach.
const lan = Object.values(networkInterfaces()).flat().find((i) =>
  i?.family === 'IPv4' && !i.internal && !i.address.startsWith('169.254.'))?.address;
if (!lan) fail('the second holder needs a non-link-local IPv4 address besides loopback');

async function freeUdpPort(host) {
  const s = createSocket('udp4');
  await new Promise((done, reject) => { s.once('error', reject); s.bind(0, host, done); });
  const port = s.address().port;
  await new Promise((done) => s.close(done));
  return port;
}

function start(cmd, argv, opts, ready) {
  return new Promise((res, rej) => {
    const c = spawn(cmd, argv, { ...opts, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    children.push(c);
    let out = '';
    let done = false;
    const timeout = setTimeout(() => finish(new Error(`${cmd} not ready:\n${out}`)), 60_000);
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

// ------------------------------------------------------------------ two holders, two places
/** The node link a node printed, with its WebRTC route. */
/** The node links a hoi-node printed, each with what it holds (the line under it): the addresses
 * themselves are packed into the link's code. */
const linksIn = (out) => [...out.matchAll(/https:\/\/\S+#\/node\?(\S+)\s+it holds (\d+) WebRTC routes? and (\d+) native routes?(, and the operator's notice, signed)?/g)]
  .map((m) => ({ query: m[1], rtc: Number(m[2]), native: Number(m[3]), notice: !!m[4] }));
const linkIn = (out) => linksIn(out).filter((l) => l.rtc).pop()?.query;

// Each holder names its own operator: one operator's nodes are one place, wherever they are.
async function holder(name, operator, host, advertise) {
  const data = mkdtempSync(join(tmpdir(), 'hoi-mailbox-'));
  const port = await freeUdpPort(host);
  // Who runs it, as the installer writes it: the node adds the sentence about mailboxes itself.
  const notice = join(data, 'notice.json');
  writeFileSync(notice, JSON.stringify({
    v: 1, node: name, operator, contact: `${operator.toLowerCase()} at example dot org`, country: 'XX', provider: 'this machine',
    processes: 'Nothing beyond what the software does.', ipLogging: false, retentionDays: 0, updated: '2026-11-04',
  }));
  const argv = ['--data', data, '--listen', '127.0.0.1:0', '--name', name, '--rtc-port', String(port), '--rtc-bind', host,
    '--hold-pieces', '0.2', '--no-port-mapping', '--notice', notice];
  if (advertise) argv.push('--rtc-advertise', `${host}:${port}`);
  const node = await start(hoiNode, argv, {}, (out) => (/holding encrypted pieces/.test(out) && linkIn(out) ? linkIn(out) : null)).catch((e) => fail(e.message));
  // Public node links now pack their routes in `code`; check the listener's printed
  // address here, then exercise the packed link through the real browser below.
  if (!node.out().split('\n').some(line => line.startsWith('WebRTC on UDP ') && line.endsWith(`@${host}:${port}`))) {
    fail(`${name} does not offer WebRTC at ${host}:${port}:\n${node.out()}`);
  }
  return { name, data, port, link: node.match, out: node.out };
}
const one = await holder('mailbox one', 'Ada', '127.0.0.1', true);
const two = await holder('mailbox two', 'Bo', lan, false);
console.log(`✓ two hoi-node holders up: WebRTC at 127.0.0.1:${one.port} and ${lan}:${two.port}, each keeping up to 0.2 GB for others`);

// ------------------------------------------------------------------ the app, nothing configured
process.env.VITE_HOI_BACKEND = 'wasm';
process.env.VITE_HOI_REGISTRY_URL = '';
process.env.VITE_HOI_RELAY_URLS = '[]';
process.env.HOI_SANDBOX = '0';
vite = await createViteServer({ root: resolve(root, 'apps/web'), logLevel: 'error', server: { host: '127.0.0.1', port: 0, hmr: false } });
await vite.listen();
const origin = `http://127.0.0.1:${vite.httpServer.address().port}`;
console.log(`✓ the app at ${origin}, with no registry and no relay`);

browser = await launch();
const errors = [];

async function call(page, method, ...args) {
  return page.evaluate(async ({ method, args }) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    return app.core[method](...args);
  }, { method, args });
}

const people = new Map();
const logs = new Map();
async function person(name) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce' });
  await consentAccepted(context);
  const page = await context.newPage();
  if (browser.browserType().name() === 'chromium') {
    // These two fixtures deliberately use this device and its LAN. Recent Chromium
    // requires the same site permissions a person grants for those addresses.
    const session = await context.newCDPSession(page);
    const { targetInfo } = await session.send('Target.getTargetInfo');
    for (const name of ['local-network', 'loopback-network', 'local-network-access']) {
      try {
        await session.send('Browser.setPermission', { permission: { name }, setting: 'granted', origin, browserContextId: targetInfo.browserContextId });
      } catch { /* Older Chromium recognizes only the combined permission. */ }
    }
    await session.detach();
  }
  people.set(name, page);
  logs.set(name, []);
  page.on('pageerror', (e) => errors.push(`${name}: ${e.message}`));
  page.on('console', (m) => logs.get(name).push(`${m.type()}: ${m.text()}`.slice(0, 300)));
  page.setDefaultTimeout(60_000);
  await page.goto(origin);
  await page.evaluate(async (name) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const deadline = Date.now() + 60_000;
    while (app.phase === 'booting' && Date.now() < deadline) await new Promise((done) => setTimeout(done, 50));
    if (app.phase === 'locked') await app.unlock('mailbox browser test passphrase');
    if (app.phase !== 'ready') throw new Error(`App did not become ready: ${app.phase}: ${app.bootError}`);
    await app.identityChanged(await app.core.createIdentity(name));
    await app.setSettings({ motion: 'reduced' });
  }, name);
  return page;
}

/** What a page's core says about its links and groups, for a failure message. */
async function state(page) {
  return page.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const net = await app.core.net();
    const groups = await app.core.privateGroups().catch((e) => e.message);
    const short = Array.isArray(groups) ? groups.map((g) => ({ epoch: g.epoch, members: g.members.length, pending: g.pending, received: g.received.length, transport: g.transport, mailboxes: g.mailboxes })) : groups;
    const outbox = Array.isArray(groups) ? await Promise.all(groups.map((g) => app.core.privateOutbox(g.community).then((o) => o.length, (e) => e.message))) : [];
    return { peers: net.peers, error: net.error, groups: short, outbox };
  }).catch((e) => e.message);
}

/** Polls `check` (a function of the page) until it returns something truthy, or fails, saying
 * what `page` saw. */
async function until(what, seconds, check, page) {
  const deadline = Date.now() + seconds * 1000;
  let last;
  while (Date.now() < deadline) {
    try {
      last = await check();
      if (last) return last;
    } catch (e) {
      last = e.message;
    }
    await new Promise((done) => setTimeout(done, 1000));
  }
  const seen = [];
  for (const [name, p] of people) {
    seen.push(`--- ${name}${p === page ? ' (waited on)' : ''}: ${JSON.stringify(await state(p))}`);
    seen.push(...(logs.get(name) ?? []).slice(-15).map((l) => `  ${l}`));
  }
  for (const h of [one, two]) {
    let held = '';
    try { held = execFileSync(hoiNode, ['admin', '--data', h.data, 'pieces'], { encoding: 'utf8' }).trim(); } catch (e) { held = e.message; }
    seen.push(`--- ${h.name}: ${held}\n${h.out().slice(-800)}`);
  }
  fail(`${what} within ${seconds} s (last: ${JSON.stringify(last)})\n${seen.join('\n')}`);
}

/** Opens a node link as the app does: its notice is read and accepted first. */
async function openNodeLink(page, query) {
  await page.evaluate(async (link) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const preview = await app.core.previewNodeLink(link);
    if (preview.notice) await app.core.acceptNotice(preview.notice.node, preview.notice.hash);
    await app.core.openNodeLink(link);
  }, `${origin}/#/node?${query}`);
}

try {
  // ---------------------------------------------------------------- mira starts a private group
  const mira = await person('mira');
  await openNodeLink(mira, one.link);
  await openNodeLink(mira, two.link);
  const community = await call(mira, 'createPrivateCommunity', { name: 'mailbox-test', description: 'Only invited devices read this.', color: 'mint', limits: { powBits: 12 } });
  const chosen = await until("mira's browser to choose two mailboxes", 180, async () => {
    const g = await call(mira, 'privateGroup', community.id);
    return g.transport === 'mailboxes' && g.mailboxes.length >= 2 && g.mailboxes.every((m) => m.linked) ? g : null;
  }, mira);
  assert.equal(chosen.mailboxes.length, 2, 'two places, two mailboxes');
  console.log("✓ mira's browser linked to both holders and made them the group's mailboxes, one per place, with no relay");

  // ---------------------------------------------------------------- kim joins with an invitation
  const kim = await person('kim');
  const request = await call(kim, 'privateJoinRequest');
  const invitation = await call(mira, 'privateInvite', community.id, request);
  await call(kim, 'privateJoin', invitation);
  const kimGroup = await call(kim, 'privateGroup', community.id);
  assert.equal(kimGroup.transport, 'mailboxes', 'the invitation carries the mailboxes');
  assert.deepEqual(kimGroup.mailboxes.map((m) => m.endpoint).sort(), chosen.mailboxes.map((m) => m.endpoint).sort());
  // Kim's browser dials the mailboxes the invitation named; each holder's notice comes first, and
  // kim reads and accepts it, as the app asks.
  const accepted = new Set();
  await until("kim's browser to link to the mailboxes", 180, async () => {
    for (const n of await call(kim, 'nodeNotices')) {
      if (n.state === 'pending') {
        await call(kim, 'acceptNotice', n.node, n.hash);
        accepted.add(n.node);
      }
    }
    return (await call(kim, 'privateGroup', community.id)).mailboxes.every((m) => m.linked);
  }, kim);
  assert.equal(accepted.size, 2, "kim was asked about both holders' notices, and nothing else");
  await until("mira's addition of kim to reach the mailboxes", 120, async () => (await call(mira, 'privateGroup', community.id)).pending === 0, mira);
  console.log("✓ kim joined with mira's invitation, and kim's browser found the mailboxes from it alone");

  // ---------------------------------------------------------------- posts both ways
  const post = await call(mira, 'createPost', { community: community.id, title: 'Only kim and I read this', text: 'Kept in mailboxes that cannot read it.' });
  await until("kim to read mira's post", 180, async () => (await call(kim, 'post', post.id))?.title === 'Only kim and I read this', kim);
  console.log("✓ kim read mira's post from the mailboxes");
  const reply = await call(kim, 'createPost', { community: community.id, title: 'Read you', text: 'Back through the same mailboxes.' });
  await until("mira to read kim's reply", 180, async () => (await call(mira, 'post', reply.id))?.title === 'Read you', mira);
  console.log("✓ mira read kim's reply the same way");
  await until('both browsers to have nothing left to send', 120, async () =>
    (await call(mira, 'privateGroup', community.id)).pending === 0 && (await call(kim, 'privateGroup', community.id)).pending === 0);

  // ---------------------------------------------------------------- what the holders keep
  for (const h of [one, two]) {
    const totals = execFileSync(hoiNode, ['admin', '--data', h.data, 'pieces'], { encoding: 'utf8' });
    const held = Number(totals.match(/pieces held: (\d+)/)?.[1] ?? 0);
    if (held < 3) fail(`${h.name} keeps ${held} sealed objects; the group wrote at least three to every mailbox:\n${totals}`);
    assert.match(totals, /none readable here/);
  }
  console.log('✓ each holder keeps the sealed messages, counted by hoi-node admin pieces, none readable there');
  const notices = await call(mira, 'nodeNotices');
  const saying = notices.filter((n) => n.sections.some((s) => s.title === 'Encrypted pieces' && /private communities/.test(s.text)));
  assert.equal(saying.length, 2, "both holders' notices say they keep private communities' messages for others");
  console.log("✓ the holders' notices say so, in the node's own sentence");

  const benign = (e) => /favicon|ERR_CONNECTION_REFUSED|WebSocket connection/.test(e);
  const real = errors.filter((e) => !benign(e));
  if (real.length) fail(`page errors:\n  ${real.join('\n  ')}`);
  console.log('✓ no page errors · relays, registries or nodes named by the app: none');
} finally {
  await browser?.close();
  await vite?.close();
}
cleanup();
process.exit(0);
