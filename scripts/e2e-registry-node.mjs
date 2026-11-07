// The registry as a node: `hoi-node --public --registry` serves the same API as the Node service
// and writes the same database. Runs the shared conformance suite over HTTP against a real
// hoi-node, with the operator's tools run from a shell (`hoi-node admin registry`) while it
// serves, stops it, then opens the database it left behind with the Node service's own store to
// prove a namespace can move between the two. A second node runs in allowlist mode.
// Run after cargo build --locked --manifest-path core/Cargo.toml -p hoi-node.
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { ALLOWED_ANNOUNCER, runAllowlistConformance, runConformance, runLogConformance, runOperatorConformance } from '../registry/conformance.mjs';
import { openRegistry } from '../registry/server.mjs';
import { nameLog } from '../registry/log.mjs';

const command = resolve('core/target/debug', process.platform === 'win32' ? 'hoi-node.exe' : 'hoi-node');

/** Starts `hoi-node --public --registry 127.0.0.1:0` on `data`; resolves once it listens. */
async function startNode(data, extra = []) {
  const child = spawn(command, [
    '--data', data, '--public', '--name', 'registry node',
    '--registry', '127.0.0.1:0', '--registry-origin', 'https://hoi.example', '--registry-origin', 'tauri://localhost',
    '--registry-limits', '10000,10000,10000',
    '--peer-bind', '127.0.0.1:0', '--rtc-port', '0', ...extra,
  ], { stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
  const node = { child, output: '', exited: once(child, 'exit') };
  child.stdout.on('data', (chunk) => { node.output += chunk; });
  child.stderr.on('data', (chunk) => { node.output += chunk; });
  for (let i = 0; i < 300 && !/registry listening on \S+/.test(node.output); i++) {
    if (child.exitCode !== null) break;
    await new Promise((done) => setTimeout(done, 100));
  }
  const listening = node.output.match(/registry listening on (\S+)/);
  assert.ok(listening, `hoi-node did not start its registry: ${node.output}`);
  node.root = `http://${listening[1]}`;
  node.stop = async () => {
    if (child.exitCode === null) child.kill('SIGINT');
    const [code] = await node.exited;
    return code;
  };
  return node;
}

const cleanup = async (data) => {
  for (const file of await readdir(data, { withFileTypes: true })) {
    if (!file.isFile()) throw new Error(`Unexpected directory in test cleanup: ${file.name}`);
  }
  await rm(data, { recursive: true, force: true });
};

const data = await mkdtemp(join(tmpdir(), 'hoi-registry-node-'));
let node;
try {
  node = await startNode(data);
  const { root } = node;
  assert.match(node.output, /public node/);
  assert.match(node.output, /carry policy: any/);
  assert.doesNotMatch(node.output, /pairing link|serving paired devices|#\/pair\?/, 'a public node has no pairing link');
  assert.match(node.output, /0 names · 0 announced nodes/);
  assert.match(node.output, /without --registry-url, node announcements are taken only when signed for this machine's loopback address/);

  const made = await runConformance(root);
  assert.equal((await (await fetch(`${root}/v1/communities?name=${made.name}`)).json()).community, made.community);
  console.log('hoi-node --registry passed the shared API conformance suite.');

  // The operator's tools work on the running registry's database from a shell.
  const admin = (...args) => execFileSync(command, ['admin', '--data', data, 'registry', ...args], { encoding: 'utf8' });
  await runOperatorConformance(root, {
    hide: async (value) => { admin('hide', value); },
    unhide: async (value) => { admin('unhide', value); },
    dropNode: async (endpoint) => { admin('drop-node', endpoint); },
  });
  assert.match(admin('list'), /refused nodes \(1\)/);
  console.log('hoi-node admin registry withheld a listing and dropped a node while the registry served.');
  await runLogConformance(root, { hide: async (value) => { admin('hide', value); } });
  const rustHead = await (await fetch(`${root}/v2/log/head`)).json();
  console.log('hoi-node --registry keeps the public log of names the same way.');

  const code = await node.stop();
  assert.equal(code, 0, `hoi-node did not stop cleanly: ${node.output}`);
  assert.doesNotMatch(node.output, /127\.0\.0\.1:\d+ (connected|joined|dialed)/, 'the node prints its own status, never a client');

  // The Node service reads what hoi-node wrote: same tables, same schema version, same rows.
  const check = new DatabaseSync(join(data, 'registry.db'), { readOnly: true });
  assert.equal(check.prepare('PRAGMA user_version').get().user_version, 5);
  assert.deepEqual(
    check.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").all().map((row) => row.name),
    ['communities', 'community_index', 'community_proofs', 'log_secret', 'name_log', 'public_nodes', 'refused_nodes', 'withheld'],
  );
  check.close();
  const registry = openRegistry(join(data, 'registry.db'));
  try {
    const found = registry.find(made.name);
    assert.equal(found.community, made.community);
    assert.equal(found.proof.version, 1);
    const release = JSON.parse(await (await import('node:fs/promises')).readFile(new URL('../release.json', import.meta.url), 'utf8'));
    const published = registry.findPublication(made.publication.name, release);
    assert.equal(published.community, made.publication.community);
    assert.equal(published.indexed, true);
    assert.equal(published.proof.version, 2);
    assert.ok(registry.listPublications('', '', release).length >= 50, 'the Node service lists what hoi-node indexed');
    assert.equal(registry.operatorView(Math.floor(Date.now() / 1000)).refusedNodes.length, 1, 'and the operator\'s lists');
    const head = nameLog(registry).head(rustHead.timestamp);
    assert.deepEqual(head, rustHead, 'the same log, key, tree and signed head');
  } finally {
    registry.close();
  }
  console.log('The Node service opened the database hoi-node --registry wrote; names, proofs, index metadata, operator lists and the log are shared.');
} catch (error) {
  console.error('hoi-node output:', node?.output);
  throw error;
} finally {
  await node?.stop();
  await cleanup(data);
}

// Allowlist mode: only the listed endpoint key's announcements are taken.
const strictData = await mkdtemp(join(tmpdir(), 'hoi-registry-node-allowlist-'));
let strict;
try {
  strict = await startNode(strictData, ['--registry-announcer', ALLOWED_ANNOUNCER.endpoint]);
  assert.match(strict.output, /allowlist mode, announcements from 1 endpoint key only/);
  await runAllowlistConformance(strict.root);
  assert.equal(await strict.stop(), 0);
  console.log('PASS: hoi-node --registry passed the conformance, operator and allowlist suites, and shares its database with the Node service.');
} catch (error) {
  console.error('hoi-node output:', strict?.output);
  throw error;
} finally {
  await strict?.stop();
  await cleanup(strictData);
}
