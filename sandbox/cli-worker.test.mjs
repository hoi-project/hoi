import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { cliArguments, discoverProviders, resolveCli, runDecision, runProcess, runWorker } from './cli-worker.mjs';

test('CLI invocations remove tool access, inherited integrations and persistent sessions', () => {
  const codex = cliArguments('codex', { outputFile: '/tmp/decision.json', model: 'chosen-model' });
  for (const arg of ['--strict-config', '--ignore-user-config', '--ignore-rules', '--ephemeral', 'features.shell_tool=false', 'features.plugins=false', 'features.hooks=false', 'features.apps=false', 'web_search="disabled"', 'mcp_servers={}', 'features.view_image=false', 'features.stable_environment_tools=false', 'features.workspace_dependencies=false', 'default_permissions="hoi-decision"', 'permissions.hoi-decision.filesystem={":root"="deny",":minimal"="read",":workspace_roots"="read"}', 'permissions.hoi-decision.network.enabled=false']) assert.ok(codex.includes(arg), arg);
  assert.ok(!codex.includes('--sandbox'), 'legacy sandbox flags would override the bounded filesystem profile');
  assert.equal(codex.at(-1), '-');
  assert.ok(!codex.some(arg => arg.includes('bypass')));
  const claude = cliArguments('claude');
  assert.equal(claude[claude.indexOf('--tools') + 1], '');
  assert.equal(claude[claude.indexOf('--disallowedTools') + 1], '*');
  for (const arg of ['--safe-mode', '--restricted', '--strict-mcp-config', '--no-session-persistence']) assert.ok(claude.includes(arg));
  assert.ok(!claude.includes('--bare'), 'subscription authentication remains usable');
});

test('Windows npm shims resolve to a JS entry point without invoking cmd.exe', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'hoi-cli-resolve-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const entry = join(directory, 'node_modules', '@anthropic-ai', 'claude-code', 'cli.js');
  await mkdir(join(directory, 'node_modules', '@anthropic-ai', 'claude-code'), { recursive: true });
  await writeFile(join(directory, 'claude.cmd'), 'do not execute this shim');
  await writeFile(entry, '// test fixture');
  const cli = await resolveCli('claude', { platform: 'win32', env: { PATH: directory } });
  assert.equal(cli.command, process.execPath); assert.deepEqual(cli.prefix, [entry]);
});

test('provider discovery only probes help and rejects old CLIs', async () => {
  const calls = [];
  const result = await discoverProviders({ resolveCommand: async driver => ({ command: driver, prefix: [] }), run: async (command, args) => { calls.push([command, args]); return { stdout: command === 'codex' ? '--strict-config --ignore-user-config --ignore-rules --ephemeral --output-last-message' : '--print' }; } });
  assert.equal(result.providers.codex.available, true);
  assert.equal(result.providers.claude.available, false);
  assert.match(result.providers.claude.error, /newer CLI/);
  assert.ok(calls.every(([, args]) => args.at(-1) === '--help'));
});

test('prompts stay on stdin; Codex and Claude outputs are normalized and temporary workspaces removed', async t => {
  const stateDir = await mkdtemp(join(tmpdir(), 'hoi-cli-turn-'));
  t.after(() => rm(stateDir, { recursive: true, force: true }));
  const maliciousText = '$(write-host secret) & echo unsafe';
  const job = { driver: 'codex', model: '', messages: [{ role: 'user', content: maliciousText }] };
  const cli = { command: 'fake', prefix: [] };
  const content = await runDecision(job, cli, { stateDir, run: async (command, args, options) => {
    assert.ok(options.input.includes(maliciousText)); assert.ok(!args.some(arg => arg.includes(maliciousText)));
    assert.deepEqual(await readdir(options.cwd), []);
    await writeFile(args[args.indexOf('--output-last-message') + 1], '{"action":"post","text":"Hi neighbours"}');
    return { stdout: '', stderr: '' };
  } });
  assert.equal(JSON.parse(content).action, 'post');
  const claude = await runDecision({ ...job, driver: 'claude' }, cli, { stateDir, run: async () => ({ stdout: JSON.stringify({ type: 'result', subtype: 'success', result: '{"action":"observe"}' }), stderr: '' }) });
  assert.equal(claude, '{"action":"observe"}');
  await assert.rejects(runDecision({ ...job, driver: 'claude' }, cli, { stateDir, run: async () => ({ stdout: '{"type":"result","subtype":"error_max_turns"}', stderr: '' }) }), /did not complete/);
  assert.deepEqual(await readdir(join(stateDir, 'cli-workspaces')), []);
});

test('process timeouts and cancellation stop the child without a shell', async () => {
  const result = await runProcess(process.execPath, ['-e', 'process.stdin.pipe(process.stdout)'], { input: 'literal $(not-a-shell)' });
  assert.equal(result.stdout, 'literal $(not-a-shell)');
  await assert.rejects(runProcess(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { timeoutMs: 40 }), /timed out/);
  const controller = new AbortController();
  const pending = runProcess(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { signal: controller.signal });
  controller.abort(); await assert.rejects(pending, { name: 'AbortError' });
  await assert.rejects(runProcess(process.execPath, ['-e', 'process.stdout.write("x".repeat(70000))']), /output exceeded/);
});

test('worker polls authenticated loopback mailbox and cancels a controller-revoked job', async () => {
  const controller = new AbortController(); let polls = 0, cancelled = false;
  const job = { id: 'job-one', driver: 'codex', model: '', messages: [] };
  await runWorker({ token: 'a'.repeat(64), signal: controller.signal,
    discover: async () => ({ providers: { codex: { available: true }, claude: { available: false, error: 'Not installed' } }, commands: { codex: {} } }),
    request: async (url, options) => {
      assert.ok(url.startsWith('http://127.0.0.1:7070/__sandbox/provider/'));
      assert.equal(options.headers['x-hoi-sandbox-token'], 'a'.repeat(64));
      assert.equal(options.redirect, 'error');
      polls++;
      return { ok: true, json: async () => ({ job: polls === 1 ? job : null, activeIds: polls === 1 ? ['job-one'] : [] }) };
    },
    decide: async (_job, _cli, { signal }) => new Promise((_, reject) => signal.addEventListener('abort', () => { cancelled = true; controller.abort(); reject(new Error('cancelled')); }, { once: true })),
  });
  assert.equal(cancelled, true); assert.equal(polls, 2);
});
