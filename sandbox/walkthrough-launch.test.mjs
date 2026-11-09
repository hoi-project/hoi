import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dockerExecutable, main, walkthroughCommand } from './walkthrough-launch.mjs';

test('walkthrough resolves Docker from fixed installation paths without searching PATH', () => {
  const checked = [];
  assert.equal(dockerExecutable({ platform: 'linux', env: { PATH: '/untrusted/bin' }, exists: candidate => {
    checked.push(candidate); return candidate === '/usr/local/bin/docker';
  } }), '/usr/local/bin/docker');
  assert.deepEqual(checked, ['/usr/bin/docker', '/usr/local/bin/docker']);
  assert.equal(dockerExecutable({ platform: 'darwin', env: {}, exists: () => true }), '/Applications/Docker.app/Contents/Resources/bin/docker');
  assert.equal(dockerExecutable({ platform: 'win32', env: {}, exists: () => true }), 'C:\\Program Files\\Docker\\Docker\\resources\\bin\\docker.exe');
  assert.throws(() => dockerExecutable({ platform: 'linux', env: { PATH: '/untrusted/bin' }, exists: () => false }), /Install Docker/);
});

test('a custom Docker installation requires an explicit existing absolute path', () => {
  assert.equal(dockerExecutable({ platform: 'linux', env: { HOI_DOCKER_BIN: '/opt/docker/bin/docker' }, exists: () => true }), '/opt/docker/bin/docker');
  for (const path of ['docker', './docker', 'bin/docker']) {
    assert.throws(() => dockerExecutable({ platform: 'linux', env: { HOI_DOCKER_BIN: path }, exists: () => true }), /absolute path/);
  }
  assert.throws(() => dockerExecutable({ platform: 'linux', env: { HOI_DOCKER_BIN: '/missing/docker' }, exists: () => false }), /absolute path/);
});

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
  assert.doesNotMatch(protocol, /^    ports:/m);
  assert.doesNotMatch(protocol, /ingress|network_mode:/);
  assert.match(gateway, /^    networks: \[ingress, walkthrough\]$/m);
  assert.match(gateway, /command: \["node", "sandbox\/walkthrough-gateway\.mjs"\]/);
  assert.match(compose, /^networks:\r?\n  ingress:\r?\n  walkthrough:\r?\n    internal: true/m);
  assert.match(compose, /127\.0\.0\.1:7080:7080/);
  assert.deepEqual(compose.match(/"127\.0\.0\.1:\d+:\d+"/g), ['"127.0.0.1:7080:7080"']);
  assert.match(compose, /internal: true/);
  assert.match(compose, /read_only: true/);
  assert.doesNotMatch(compose, /docker\.sock|host\.docker\.internal|network_mode:|privileged:|providers|cli-worker/);
});
