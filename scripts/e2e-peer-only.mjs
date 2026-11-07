// Run after cargo build --locked --manifest-path core/Cargo.toml -p hoi-node.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer, createConnection } from 'node:net';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { once } from 'node:events';
const probe = createServer();
probe.listen(0, '127.0.0.1');
await once(probe, 'listening');
const port = probe.address().port;
await new Promise(done => probe.close(done));
const data = await mkdtemp(join(tmpdir(), 'hoi-peer-only-'));
const child = spawn(resolve('core/target/debug/hoi-node'), ['--data', data, '--peer-only', '--listen', `127.0.0.1:${port}`, '--peer-bind', '127.0.0.1:0', '--relaying', '--no-port-mapping'], { stdio: ['ignore', 'pipe', 'pipe'] });
let output = '';
child.stdout.on('data', chunk => { output += chunk; });
child.stderr.on('data', chunk => { output += chunk; });
const exited = once(child, 'exit');
try {
  for (let i = 0; i < 100 && !output.includes('relaying for members'); i++) {
    if (child.exitCode !== null) break;
    await new Promise(done => setTimeout(done, 100));
  }
  assert.match(output, /relaying for members' browsers and nodes: on/);
  assert.match(output, /WebRTC on UDP/);
  assert.doesNotMatch(output, /pairing link|serving paired devices|#\/pair\?/);
  const result = await new Promise(resolve => {
    const socket = createConnection({ host: '127.0.0.1', port });
    socket.once('connect', () => { socket.destroy(); resolve('connected'); });
    socket.once('error', error => resolve(error.code));
    socket.setTimeout(2000, () => { socket.destroy(); resolve('timeout'); });
  });
  assert.equal(result, 'ECONNREFUSED', 'peer-only must not start the control listener');
  child.kill('SIGINT');
  const [code] = await exited;
  assert.equal(code, 0);
  console.log('PASS: native peer-only mode relays, exposes no control listener or pairing link, and shuts down cleanly.');
} finally {
  if (child.exitCode === null) { child.kill('SIGINT'); await exited; }
  await rm(data, { recursive: true, force: true });
}
