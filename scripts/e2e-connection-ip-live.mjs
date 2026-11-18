// Chromium contacts only a fresh local hoi-node, once over WebSocket and once over pinned
// WebRTC. The same browser then pairs and reads its live per-connection address in WASM.
// Run after building hoi-node and the web WASM. HOI_NODE and HOI_WASM_PKG override those paths.
// HOI_IP_TEST_HOST can select a numeric local interface address for the RTC path.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createSocket } from 'node:dgram';
import { once } from 'node:events';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { chromium } from 'playwright';
import { startComponentServer } from './e2e-component-server.mjs';

const root = resolve(import.meta.dirname, '..');
const binary = process.env.HOI_NODE || resolve(root, 'core/target/debug/hoi-node');
const pkg = process.env.HOI_WASM_PKG || resolve(root, 'apps/web/src/lib/core/wasm/pkg');
const data = await mkdtemp(join(tmpdir(), 'hoi-connection-ip-'));
const rtcHost = process.env.HOI_IP_TEST_HOST || '127.0.0.1';
const socket = createSocket('udp4');
socket.bind(0, rtcHost);
await once(socket, 'listening');
const port = socket.address().port;
socket.close();

const { server, origin } = await startComponentServer('/connection-ip-live', '<!doctype html><html lang="en"><title>Connection test</title></html>');
const node = spawn(binary, [
  '--data', data, '--listen', '127.0.0.1:0', '--peer-bind', '127.0.0.1:0',
  '--rtc-bind', rtcHost, '--rtc-advertise', `${rtcHost}:${port}`, '--rtc-port', String(port),
  '--no-lan', '--no-port-mapping', '--relaying', '--name', 'local test home',
], { env: { ...process.env, HOI_ALLOWED_ORIGINS: origin }, stdio: 'ignore' });
let launchError;
node.once('error', error => { launchError = error; });

async function within(promise, timeout, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), timeout);
    })]);
  } finally { clearTimeout(timer); }
}

let browser;
try {
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const page = await browser.newPage();
  for (const [file, contentType] of [['hoi_wasm.js', 'text/javascript'], ['hoi_wasm_bg.wasm', 'application/wasm']]) {
    await page.route(`${origin}/fixture-wasm/${file}`, request => request.fulfill({ path: join(pkg, file), contentType }));
  }
  await page.goto(`${origin}/connection-ip-live`);
  const client = await page.evaluateHandle(async () => {
    const { default: init, WasmNode } = await import('/fixture-wasm/hoi_wasm.js');
    await init();
    return new WasmNode('web', location.origin + '/', [], []);
  });
  let link, preview;
  for (let i = 0; i < 600; i++) {
    if (launchError) throw launchError;
    if (node.exitCode !== null) throw new Error('Local hoi-node exited during startup.');
    try {
      link = (await readFile(join(data, 'pairing-link'), 'utf8')).trim().split('\n')[0];
      preview = await client.evaluate((node, link) => JSON.parse(node.call('previewPair', JSON.stringify([link]))), link);
      if (preview.routes.some(r => r.endsWith(`@${rtcHost}:${port}`))) break;
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
    await delay(200);
  }
  const rtc = preview?.routes.find(r => r.endsWith(`@${rtcHost}:${port}`));
  assert.ok(rtc, 'The local node published its selected RTC route.');
  const cdp = await page.context().newCDPSession(page);
  const { targetInfo } = await cdp.send('Target.getTargetInfo');
  for (const name of ['local-network', 'loopback-network', 'local-network-access']) {
    try { await cdp.send('Browser.setPermission', { permission: { name }, setting: 'granted', origin, browserContextId: targetInfo.browserContextId }); }
    catch { /* Older Chromium uses the combined permission. */ }
  }
  await cdp.detach();

  for (const [target, transport] of [[preview.url, 'websocket'], [`rtc:${rtc}`, 'webrtc']]) {
    const observed = await page.evaluate(async route => {
      const { checkConnection } = await import('/src/lib/core/connection-check.ts');
      return checkConnection(route, new AbortController().signal);
    }, target);
    assert.deepEqual(observed, { address: transport === 'webrtc' ? rtcHost : '127.0.0.1', transport });
    console.log(`✓ Chromium received its socket IP over ${transport} from hoi-node.`);
  }

  // No probe sent a pairing credential. The original link can still be spent here.
  const report = client.evaluate(async (node, { link, url }) => {
    node.call('pair', JSON.stringify([link]));
    node.net_connect('home', 'home');
    node.set_peer_meta('home', 'websocket', true, false);
    const socket = new WebSocket(url);
    socket.binaryType = 'arraybuffer';
    try {
      return await new Promise((resolve, reject) => {
        socket.onopen = () => {
          for (const [, bytes] of node.take_outbox()) socket.send(bytes);
        };
        socket.onerror = socket.onclose = () => reject(new Error('Paired WebSocket closed.'));
        socket.onmessage = event => {
          try {
            if (!node.net_receive('home', new Uint8Array(event.data))) throw new Error('Paired WASM client rejected the home hello.');
            const status = JSON.parse(node.call('net', '[]'));
            if (status.homeNode?.connected) resolve(status.connections.find(c => c.role === 'home')?.observedAddress);
          } catch (error) { reject(error); }
        };
      });
    } finally { socket.close(); }
  }, { link, url: preview.url });
  assert.equal(await within(report, 10_000, 'No authenticated home address observation arrived.'), '127.0.0.1');
  console.log('✓ The paired browser received its source IP in the authenticated home hello.');
} finally {
  await browser?.close();
  if (node.pid && node.exitCode === null) {
    const stopped = once(node, 'exit');
    node.kill('SIGTERM');
    await within(stopped, 5000, 'Local node did not shut down.').catch(() => node.kill('SIGKILL'));
  }
  await server.close();
  await rm(data, { recursive: true, force: true });
}
