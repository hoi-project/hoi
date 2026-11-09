import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { main, walkthroughCommand } from './walkthrough-launch.mjs';

test('walkthrough startup cannot build, pull, or launch an AI worker', async () => {
  const calls = [];
  await main([], async args => calls.push(args));
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].slice(5), ['up', '--detach', '--no-build', '--pull', 'never', '--wait', '--wait-timeout', '90']);
  assert.equal(walkthroughCommand('install').at(-1), 'build');
  assert.equal(walkthroughCommand('stop').at(-1), 'stop');
  assert.throws(() => walkthroughCommand('down'));
  await assert.rejects(main(['stop', 'all']));
});

test('only the fixed observation gateway joins ingress; protocol processes stay on the internal network', async () => {
  const compose = await readFile(new URL('./walkthrough-compose.yml', import.meta.url), 'utf8');
  const protocol = compose.match(/^  walkthrough:\r?\n([\s\S]*?)(?=^  gateway:)/m)?.[1];
  const gateway = compose.match(/^  gateway:\r?\n([\s\S]*?)(?=^networks:)/m)?.[1];
  assert.ok(protocol && gateway, 'the protocol and ingress services remain separate');
  assert.match(protocol, /^    networks: \[walkthrough\]$/m);
  assert.doesNotMatch(protocol, /^    ports:|ingress|network_mode:/m);
  assert.match(gateway, /^    networks: \[ingress, walkthrough\]$/m);
  assert.match(gateway, /command: \["node", "sandbox\/walkthrough-gateway\.mjs"\]/);
  assert.match(compose, /^networks:\r?\n  ingress:\r?\n  walkthrough:\r?\n    internal: true/m);
  assert.match(compose, /127\.0\.0\.1:7080:7080/);
  assert.deepEqual(compose.match(/"127\.0\.0\.1:\d+:\d+"/g), ['"127.0.0.1:7080:7080"']);
  assert.match(compose, /internal: true/);
  assert.match(compose, /read_only: true/);
  assert.doesNotMatch(compose, /docker\.sock|host\.docker\.internal|network_mode:|privileged:|providers|cli-worker/);
});
