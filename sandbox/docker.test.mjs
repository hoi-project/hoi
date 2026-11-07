import test from 'node:test';
import assert from 'node:assert/strict';
import { Docker, DockerError, decodeFrames } from './docker.mjs';

function frame(text, stream = 1) {
  const data = Buffer.from(text);
  const header = Buffer.alloc(8);
  header[0] = stream;
  header.writeUInt32BE(data.length, 4);
  return Buffer.concat([header, data]);
}

function fixture({ output = { ok: true }, exit = 0, start, oomKilled = false } = {}) {
  const calls = [];
  let running = true;
  const docker = new Docker({ project: 'test-sandbox', transport: async (method, path, body) => {
    calls.push({ method, path, body });
    if (path.endsWith('/json') && path.startsWith('/containers/')) {
      return { Config: { Labels: docker.labels('alice') }, HostConfig: { NanoCpus: 2_000_000_000 }, State: { Running: running, OOMKilled: oomKilled } };
    }
    if (path.endsWith('/exec')) return { Id: 'exec-1' };
    if (path === '/exec/exec-1/start') return start ? start() : frame(JSON.stringify(output));
    if (path === '/exec/exec-1/json') return { Running: false, ExitCode: exit };
    if (path.endsWith('/stop?t=1')) { running = false; return null; }
    if (path.endsWith('/start')) { running = true; return null; }
    throw new Error(`Unexpected request ${method} ${path}`);
  } });
  return { docker, calls };
}

test('Docker output keeps JSON stdout separate from diagnostic stderr', () => {
  assert.deepEqual(decodeFrames(Buffer.concat([frame('{"ok":'), frame('warning\n', 2), frame('true}')])), { stdout: '{"ok":true}', stderr: 'warning\n' });
  const incomplete = frame('hello').subarray(0, 10);
  assert.throws(() => decodeFrames(incomplete), /truncated/);
});

test('foreign container names cannot be adopted or stopped', async () => {
  const docker = new Docker({ transport: async () => ({ Config: { Labels: {} }, State: { Running: true } }) });
  await assert.rejects(docker.ensureMember('alice'), /outside this sandbox/);
  await assert.rejects(docker.stopMember('alice'), /outside this sandbox/);
  await assert.rejects(docker.ensureMember('../foreign'), /Member IDs/);
});

test('agent creation refuses a network with outbound access', async () => {
  const docker = new Docker({ transport: async (_method, path) => {
    if (path.startsWith('/containers/')) throw new DockerError('missing', 404);
    return { Internal: false };
  } });
  await assert.rejects(docker.ensureMember('alice'), /network must be internal/);
});

test('new agents receive bounded CPU and memory before identity creation', async () => {
  const calls = [];
  const docker = new Docker({ transport: async (method, path, body) => {
    calls.push({ method, path, body });
    if (method === 'GET' && path.startsWith('/containers/')) throw new DockerError('missing', 404);
    if (path.startsWith('/networks/')) return { Internal: true };
    return null;
  } });
  await docker.ensureMember('alice');
  const created = calls.find(c => c.path.startsWith('/containers/create?'));
  assert.equal(created.body.HostConfig.NanoCpus, 2_000_000_000);
  assert.equal(created.body.HostConfig.Memory, 512 * 1024 * 1024);
});

test('existing owned agents adopt the CPU quota without replacing their identity volume', async () => {
  const calls = [];
  const docker = new Docker({ transport: async (method, path, body) => {
    calls.push({ method, path, body });
    if (method === 'GET') return { Config: { Labels: docker.labels('alice') }, HostConfig: { NanoCpus: 0 }, State: { Running: true } };
    return null;
  } });
  await docker.ensureMember('alice');
  assert.deepEqual(calls.find(c => c.path.endsWith('/update')).body, { NanoCpus: 2_000_000_000, Memory: 512 * 1024 * 1024 });
  assert.ok(calls.every(c => !c.path.includes('/create') && c.method !== 'DELETE'));
});

test('rebuilt images replace stale containers while retaining the same identity volume', async () => {
  const calls = [];
  const docker = new Docker({ project: 'test-sandbox', transport: async (method, path, body) => {
    calls.push({ method, path, body });
    if (method === 'GET' && path.startsWith('/containers/')) return {
      Image: 'sha256:old', Config: { Labels: docker.labels('alice') },
      HostConfig: { NanoCpus: 2_000_000_000 }, State: { Running: false },
    };
    if (path.startsWith('/images/')) return { Id: 'sha256:new' };
    if (path.startsWith('/networks/')) return { Internal: true };
    return null;
  } });
  await docker.ensureMember('alice');
  assert.deepEqual(calls.filter(c => c.method === 'DELETE').map(c => c.path), ['/containers/test-sandbox-agent-alice']);
  const created = calls.find(c => c.path.startsWith('/containers/create?'));
  assert.equal(created.body.Image, 'hoi-sandbox:latest');
  assert.deepEqual(created.body.HostConfig.Binds, ['test-sandbox-agent-alice-data:/data']);
  assert.ok(calls.every(c => !(c.method === 'DELETE' && c.path.startsWith('/volumes/'))));
});

test('containers built from the current image are reused', async () => {
  const calls = [];
  const docker = new Docker({ transport: async (method, path) => {
    calls.push({ method, path });
    if (path.startsWith('/images/')) return { Id: 'sha256:current' };
    return { Image: 'sha256:current', Config: { Labels: docker.labels('alice') }, HostConfig: { NanoCpus: 2_000_000_000 }, State: { Running: true } };
  } });
  await docker.ensureMember('alice');
  assert.ok(calls.every(c => c.method === 'GET'));
});

test('actions send literal arguments without invoking a shell', async () => {
  const { docker, calls } = fixture();
  const text = 'literal $(echo no) ; & | text';
  await docker.action('alice', ['post', 'community', 'title', text]);
  const exec = calls.find(c => c.path.endsWith('/exec'));
  assert.equal(exec.body.Cmd[0], 'hoi-sim');
  assert.equal(exec.body.Cmd.at(-1), text);
  assert.throws(() => docker.action('alice', ['sh', '-c', 'echo no']), /Invalid agent action/);
});

test('JSON failures are failures even when a protocol action exits zero', async () => {
  const { docker } = fixture({ output: { ok: false, error: 'not allowed' } });
  await assert.rejects(docker.action('alice', ['post', 'community', 'title', 'text']), /not allowed/);
});

test('OOM kills explain the memory budget instead of reporting unusable JSON', async () => {
  const { docker } = fixture({ exit: 137, oomKilled: true, start: () => Buffer.alloc(0) });
  await assert.rejects(docker.action('alice', ['init', '--name', 'Alice']), /exceeded its 512 MiB memory budget/);
});

test('forgery findings survive the intentionally failing process exit', async () => {
  const evidence = { ok: true, all_held_locally: false, forgeries: [{ held: false }] };
  const { docker } = fixture({ output: evidence, exit: 1 });
  assert.deepEqual(await docker.action('alice', ['forge', 'bad-sig', 'community']), evidence);
});

test('cancelling a running action stops the actual container', async () => {
  let started;
  const ready = new Promise(resolve => { started = resolve; });
  const { docker, calls } = fixture({ start: () => { started(); return new Promise(() => {}); } });
  const controller = new AbortController();
  const result = docker.action('alice', ['sync'], { signal: controller.signal });
  await ready;
  controller.abort();
  await assert.rejects(result, error => error.name === 'AbortError');
  assert.ok(calls.some(c => c.method === 'POST' && c.path.endsWith('/stop?t=1')));
});
