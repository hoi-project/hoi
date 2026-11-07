import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { buildBundle, parseBuildArgs, validateBinary } from './build-server.mjs';
import { BUNDLE_FILES, configureOptions, controlService, deployBundle, parseDeployArgs, serviceCommand, serviceUnits, canShareConnection } from '../deploy/server/bundle/deploy.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const bash = process.platform === 'win32' ? join(process.env.ProgramFiles || 'C:/Program Files', 'Git/bin/bash.exe') : 'bash';
function temporary(t) {
  const parent = resolve(tmpdir());
  const dir = mkdtempSync(join(parent, 'hoi-bundle-test-'));
  t.after(() => {
    assert.equal(dirname(dir), parent);
    assert.ok(basename(dir).startsWith('hoi-bundle-test-'));
    rmSync(dir, { recursive: true, force: true });
  });
  return dir;
}
function elf(arch = 'x64') {
  const bytes = Buffer.alloc(64);
  Buffer.from([127, 69, 76, 70, 2, 1]).copy(bytes);
  bytes.writeUInt16LE(arch === 'x64' ? 62 : 183, 18);
  return bytes;
}
function command(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, { encoding: 'utf8', ...options });
  assert.equal(result.status, 0, `${cmd}: ${result.error ?? ''}\n${result.stdout}\n${result.stderr}`);
  return result.stdout;
}

test('build arguments reject unsupported architectures, missing values and unknown flags', () => {
  assert.equal(parseBuildArgs([]).arch, 'x64');
  assert.equal(parseBuildArgs(['--arch', 'arm64']).arch, 'arm64');
  for (const args of [['--arch', 'win32'], ['--arch', '__proto__'], ['--out'], ['--binary', '--arch'], ['--unknown']]) {
    assert.throws(() => parseBuildArgs(args));
  }
  assert.throws(() => validateBinary(Buffer.from('MZ windows executable'), 'x64'), /Linux ELF/);
  assert.throws(() => validateBinary(elf('arm64'), 'x64'), /does not match/);
});

test('bundle round-trips through tar with a binary, installer, checksums and preserved settings', t => {
  const dir = temporary(t);
  const binary = join(dir, 'hoi-node');
  const out = join(dir, 'output with spaces');
  writeFileSync(binary, elf());
  const result = buildBundle({ arch: 'x64', binary, out });
  const unpack = join(dir, 'unpacked');
  mkdirSync(unpack);
  command('tar', ['-xzf', result.archive, '-C', unpack]);
  const extracted = join(unpack, 'hoi-server-linux-x64');
  assert.deepEqual(readdirSync(extracted).sort(), [...BUNDLE_FILES].sort());
  assert.deepEqual(readFileSync(join(extracted, 'hoi-node')), elf());
  for (const line of readFileSync(join(extracted, 'SHA256SUMS'), 'utf8').trim().split('\n')) {
    const [hash, file] = line.split('  ');
    assert.equal(hash, createHash('sha256').update(readFileSync(join(extracted, file))).digest('hex'));
  }
  for (const file of ['install.sh', 'node-installer.sh']) assert.ok(!readFileSync(join(extracted, file), 'utf8').includes('\r'));
  assert.doesNotMatch(readFileSync(join(extracted, 'install.sh'), 'utf8'), /@ARCH@/);
  // The copied deploy helper works without imports from the repository.
  assert.match(command(process.execPath, [join(extracted, 'deploy.mjs'), '--help'], { cwd: dir }), /USER@HOST/);
  const config = 'role=public\nname=custom label\n';
  writeFileSync(join(result.directory, 'server.conf'), config);
  buildBundle({ arch: 'x64', binary, out });
  assert.equal(readFileSync(join(result.directory, 'server.conf'), 'utf8'), config);
  command('tar', ['-xzf', result.archive, '-C', unpack]);
  assert.equal(readFileSync(join(extracted, 'server.conf'), 'utf8'), config);
  assert.ok(!readdirSync(out).some(file => file.startsWith('.build-')));
});

test('existing release archives work for ARM64; failed builds preserve previous output', t => {
  const dir = temporary(t);
  const binary = join(dir, 'hoi-node');
  writeFileSync(binary, elf('arm64'));
  const archive = join(dir, 'release.tar.gz');
  command('tar', ['-czf', archive, '-C', dir, 'hoi-node']);
  const options = { arch: 'arm64', binary: archive, out: join(dir, 'out') };
  const result = buildBundle(options);
  const before = readFileSync(result.archive);
  writeFileSync(binary, elf());
  assert.throws(() => buildBundle({ ...options, binary }), /does not match/);
  assert.deepEqual(readFileSync(result.archive), before);
  assert.deepEqual(readFileSync(join(result.directory, 'hoi-node')), elf('arm64'));
  assert.ok(!readdirSync(options.out).some(file => file.startsWith('.build-')));
});

test('SSH destinations and port values cannot become command options or shell commands', () => {
  for (const host of ['-oProxyCommand=bad', 'host;touch bad', 'user@host:22', 'user@host\nreboot', '$(reboot)']) {
    assert.throws(() => parseDeployArgs([host]));
  }
  for (const port of ['0', '65536', '22;reboot', 'abc']) assert.throws(() => parseDeployArgs(['host', '--port', port]));
  assert.equal(parseDeployArgs(['my-vps']).host, 'my-vps');
  assert.equal(parseDeployArgs(['root@[2001:db8::1]']).host, 'root@[2001:db8::1]');
  assert.throws(() => parseDeployArgs(['host', '--identity']));
});

test('SSH upload quotes installer arguments, uses a TTY and stops after a transfer failure', t => {
  const dir = temporary(t);
  for (const file of BUNDLE_FILES) writeFileSync(join(dir, file), 'fixture');
  const identity = join(dir, 'key with spaces');
  writeFileSync(identity, 'fixture key');
  const label = "a label's $(touch injected); `touch injected`";
  const options = parseDeployArgs(['user@my-vps', '--bundle', dir, '--port', '2222', '--identity', identity, '--', '--name', label]);
  const calls = [];
  deployBundle(options, (cmd, args, capture) => {
    calls.push({ cmd, args, capture });
    return capture ? '/tmp/hoi-deploy.Abc123\n' : '';
  });
  // One shared connection on Linux and macOS (ControlMaster), closed at the end; none on Windows.
  const shared = canShareConnection();
  assert.deepEqual(calls.map(c => c.cmd), shared ? ['tar', 'ssh', 'scp', 'ssh', 'ssh'] : ['tar', 'ssh', 'scp', 'ssh']);
  assert.deepEqual(calls[1].args.slice(0, 4), ['-p', '2222', '-i', identity]);
  assert.ok(calls[1].args.includes('-n'), 'the first command takes nothing typed at the prompt');
  assert.deepEqual(calls[2].args.slice(0, 4), ['-P', '2222', '-i', identity]);
  assert.equal(calls[2].args.at(-1), 'user@my-vps:/tmp/hoi-deploy.Abc123/bundle.tar.gz');
  assert.deepEqual(calls[3].args.slice(0, 4), ['-p', '2222', '-i', identity]);
  assert.deepEqual(calls[3].args.slice(-3, -1), ['-t', 'user@my-vps']);
  for (const call of calls.slice(1, 4)) {
    const controls = call.args.filter((a, i) => call.args[i - 1] === '-o');
    if (shared) {
      assert.ok(controls.includes('ControlMaster=auto') && controls.includes('ControlPersist=60'), `${call.cmd} shares the connection`);
      const path = controls.find(c => c.startsWith('ControlPath='));
      assert.match(path, /hoi-ssh-[^/]+\/cm-%C$/);
      assert.ok(path.length < 90, `socket path stays short: ${path}`);
    } else assert.deepEqual(controls, []);
  }
  if (shared) assert.deepEqual(calls[4].args.slice(-3), ['-O', 'exit', 'user@my-vps']);
  const remoteCommand = calls[3].args.at(-1);
  // Execute just the quoted argument list in a real shell, never a remote install.
  const quotedFlags = remoteCommand.split('bash ./install.sh ')[1];
  const echoed = command(bash, ['-c', `printf '%s\\0' ${quotedFlags}`], { cwd: dir });
  assert.deepEqual(echoed.split('\0'), ['--role', 'config', '--name', label, '']);
  assert.ok(!existsSync(join(dir, 'injected')));
  assert.ok(!existsSync(dirname(calls[0].args[1])), 'temporary archive directory removed');
  const failed = [];
  assert.throws(() => deployBundle(options, (cmd, args, capture) => {
    failed.push(cmd);
    if (cmd === 'scp') throw new Error('transfer failed');
    return capture ? '/tmp/hoi-deploy.Abc123\n' : '';
  }), /transfer failed/);
  assert.deepEqual(failed, shared ? ['tar', 'ssh', 'scp', 'ssh'] : ['tar', 'ssh', 'scp'], 'after a failure the shared connection is still closed');
  assert.throws(() => deployBundle(options, (_cmd, _args, capture) => capture ? '/tmp/unsafe;reboot' : ''), /valid upload directory/);
});

test('an operator notice travels with the bundle, and the menu asks the operator to acknowledge', async t => {
  const dir = temporary(t);
  for (const file of BUNDLE_FILES) writeFileSync(join(dir, file), 'fixture');
  const notice = join(dir, 'my notice.json');
  writeFileSync(notice, '{"v":1,"operator":"Lena"}');
  const options = parseDeployArgs(['user@my-vps', '--bundle', dir, '--role', 'public', '--notice', notice]);
  assert.equal(options.notice, notice);
  const calls = [];
  deployBundle(options, (cmd, args, capture) => {
    calls.push({ cmd, args });
    return capture ? '/tmp/hoi-deploy.Abc123\n' : '';
  });
  const tar = calls[0].args;
  assert.equal(tar.at(-1), 'notice.json');
  assert.ok(tar.filter(a => a === '-C').length === 2 && tar.at(-3) === '-C', 'the notice is packed from its own folder');
  assert.match(calls[3].args.at(-1), /'--notice-file' '\.\/notice\.json'/);
  assert.throws(() => parseDeployArgs(['user@my-vps', '--action', 'stop', '--notice', notice]), /goes with a deployment/);
  assert.throws(() => deployBundle(parseDeployArgs(['user@my-vps', '--bundle', dir, '--role', 'public', '--notice', join(dir, 'missing.json')]), () => ''), /Notice file not found/);
  // The menu: a public node asks for the acknowledgement and the notice; "n" changes nothing.
  const answers = ['deploy', 'public', 'n'];
  await assert.rejects(configureOptions(parseDeployArgs(['user@my-vps', '--bundle', dir]), async () => answers.shift()), /Nothing was changed/);
  const yes = ['deploy', 'public', 'yes', notice, 'keep'];
  const configured = await configureOptions(parseDeployArgs(['user@my-vps', '--bundle', dir]), async () => yes.shift());
  assert.equal(configured.notice, notice);
  assert.equal(configured.role, 'public');
  const none = ['deploy', 'public', 'y', '', 'keep'];
  assert.equal((await configureOptions(parseDeployArgs(['user@my-vps', '--bundle', dir]), async () => none.shift())).notice, undefined);
  const home = ['deploy', 'home', 'keep'];
  assert.equal((await configureOptions(parseDeployArgs(['user@my-vps', '--bundle', dir]), async () => home.shift())).role, 'home', 'a home node is not asked');
});

test('bundle installer resolves its own paths and refuses wrong CPUs and corrupted files', t => {
  const dir = temporary(t);
  const payload = join(dir, 'payload space');
  mkdirSync(payload);
  mkdirSync(join(dir, 'bin'));
  const script = (path, body) => { writeFileSync(path, body); chmodSync(path, 0o755); };
  const uname = join(dir, 'bin/uname');
  script(uname, '#!/usr/bin/env bash\nif [ "$1" = -s ]; then echo Linux; else echo x86_64; fi\n');
  script(join(payload, 'hoi-node'), '#!/usr/bin/env bash\necho "hoi-node 1.0.0-preview.1"\n');
  script(join(payload, 'node-installer.sh'), '#!/usr/bin/env bash\nprintf "ARGS\\n"\nprintf "%s\\0" "$@"\n');
  writeFileSync(join(payload, 'server.conf'), 'role=public\n');
  writeFileSync(join(payload, 'install.sh'), readFileSync(join(root, 'deploy/server/bundle/install.sh'), 'utf8').replaceAll('\r\n', '\n').replaceAll('@ARCH@', 'x86_64'));
  writeFileSync(join(payload, 'SHA256SUMS'), ['hoi-node', 'node-installer.sh'].map(file =>
    `${createHash('sha256').update(readFileSync(join(payload, file))).digest('hex')}  ${file}\n`).join(''));
  const run = (args = ['check', '--name', 'my node']) => spawnSync(bash, ['-c', 'export PATH="$PWD/bin:$PATH"; bash "./payload space/install.sh" "$@"', 'test', ...args], { cwd: dir, encoding: 'utf8' });
  const ok = run();
  assert.equal(ok.status, 0, `${ok.error ?? ''}\n${ok.stdout}\n${ok.stderr}`);
  const args = ok.stdout.split('ARGS\n')[1].split('\0');
  assert.equal(args[0], '--config');
  assert.ok(args[1].endsWith('/payload space/server.conf'));
  assert.equal(args[2], '--binary');
  assert.ok(args[3].endsWith('/payload space/hoi-node'));
  assert.deepEqual(args.slice(4), ['--yes', 'check', '--name', 'my node', '']);
  // Selecting a role does not inherit stale registry/TLS settings from server.conf.
  writeFileSync(join(payload, 'server.conf'), 'role=registry\nregistry_url=https://old.example\ntls=caddy\n');
  for (const role of ['public', 'registry', 'home', 'private']) {
    const selected = run(['--role', role, '--name', 'selected']);
    assert.equal(selected.status, 0, selected.stderr);
    const passed = selected.stdout.split('ARGS\n')[1].split('\0');
    assert.equal(passed[0], role);
    assert.ok(!passed.includes('--config'));
  }
  writeFileSync(join(payload, 'hoi-node'), 'corrupted');
  const corrupt = run();
  assert.notEqual(corrupt.status, 0);
  assert.doesNotMatch(corrupt.stdout, /ARGS/);
  script(uname, '#!/usr/bin/env bash\nif [ "$1" = -s ]; then echo Linux; else echo aarch64; fi\n');
  const wrongCpu = run();
  assert.notEqual(wrongCpu.status, 0);
  assert.match(wrongCpu.stderr, /matching --arch/);
});

test('the menu selects registry, HTTPS and preservation; scripted commands need an explicit role', async () => {
  // A registry is a public node: the menu asks the operator to acknowledge, and for the notice.
  const answers = ['deploy', 'registry', 'https://names.example.org', 'caddy', 'y', '', 'keep'];
  const configured = await configureOptions(parseDeployArgs(['host']), async () => answers.shift());
  assert.equal(configured.role, 'registry');
  assert.equal(configured.tls, 'caddy');
  assert.equal(configured.registryUrl, 'https://names.example.org');
  assert.equal(configured.resetData, false);
  assert.deepEqual(answers, []);
  await assert.rejects(configureOptions(parseDeployArgs(['host'])), /Choose --action/);
  await assert.rejects(configureOptions(parseDeployArgs(['host', '--action', 'deploy'])), /Choose --role/);
  await assert.rejects(configureOptions(parseDeployArgs(['host', '--role', 'registry'])), /needs --registry-url/);
  const scripted = await configureOptions(parseDeployArgs(['host', '--role', 'registry', '--registry-url', 'https://names.example.org', '--tls', 'external']));
  assert.equal(scripted.tls, 'external');
  const answers2 = ['stop', 'caddy'];
  const stop = await configureOptions(parseDeployArgs(['host']), async () => answers2.shift());
  assert.equal(stop.action, 'stop');
  assert.equal(stop.service, 'caddy');
});

test('a fresh deployment requires the confirmation matching the selected role', async () => {
  for (const [role, state] of [['public', 'hoi-public'], ['registry', 'hoi-public'], ['home', 'hoi-node'], ['private', 'hoi-node']]) {
    const extra = role === 'registry' ? ['--registry-url', 'https://names.example.org'] : [];
    const argv = ['host', '--role', role, ...extra, '--reset-data'];
    await assert.rejects(configureOptions(parseDeployArgs(argv)), new RegExp(`--confirm-reset ${state}`));
    await assert.rejects(configureOptions(parseDeployArgs([...argv, '--confirm-reset', 'wrong'])), /nothing was changed/);
    const options = await configureOptions(parseDeployArgs([...argv, '--confirm-reset', state]));
    assert.equal(options.resetData, true);
    assert.equal(options.confirmReset, state);
  }
  for (const argv of [
    ['host', '--action', 'stop', '--reset-data'],
    ['host', '--action', 'check', '--reset-data'],
    ['host', '--confirm-reset', 'hoi-public'],
    ['host', '--action', 'stop', '--', '--name', 'oops'],
  ]) assert.throws(() => parseDeployArgs(argv));
});

test('service controls need no bundle or upload and accept only exact service names', t => {
  const dir = temporary(t);
  const options = parseDeployArgs(['host', '--action', 'stop', '--service', 'registry', '--bundle', join(dir, 'missing')]);
  const calls = [];
  controlService(options, (cmd, args) => calls.push({ cmd, args }));
  assert.equal(calls.length, 1);
  assert.equal(calls[0].cmd, 'ssh');
  assert.ok(calls[0].args.includes('-t'));
  assert.match(calls[0].args.at(-1), /systemctl stop/);
  assert.doesNotMatch(calls[0].args.at(-1), /clean|rm |install\.sh|scp/);
  assert.deepEqual(serviceUnits('registry'), ['hoi-node.service']);
  assert.deepEqual(serviceUnits('old-registry'), ['old-registry.service']);
  for (const service of ['*', 'hoi;reboot', '../ssh', '-a', 'other.target']) assert.throws(() => serviceUnits(service));
});

test('service command execution stops/disables named units and filters absent optional services', t => {
  const dir = temporary(t);
  mkdirSync(join(dir, 'bin'));
  const script = (file, text) => { writeFileSync(join(dir, 'bin', file), text); chmodSync(join(dir, 'bin', file), 0o755); };
  script('id', '#!/usr/bin/env bash\necho 0\n');
  script('systemctl', `#!/usr/bin/env bash
if [ "$1" = show ] && [ "$2" = --property=LoadState ]; then
  if [ "$4" = hoi-node.service ]; then echo loaded; else echo not-found; fi
else printf '%s\\0' "$@"; fi
`);
  const run = (action, service) => command(bash, ['-c', `export PATH="$PWD/bin:$PATH"; ${serviceCommand({ action, service, installArgs: [] })}`], { cwd: dir }).split('\0');
  assert.deepEqual(run('stop', 'registry'), ['stop', '--', 'hoi-node.service', '']);
  assert.deepEqual(run('disable', 'caddy'), ['disable', '--now', '--', 'caddy.service', '']);
  assert.deepEqual(run('start', 'all'), ['start', '--', 'hoi-node.service', '']);
  assert.deepEqual(run('services'), ['list-units', '--type=service', '--all', '--no-pager', '']);
});

test('the real installer keeps data by default, validates resets, and stops before cleaning', t => {
  const dir = temporary(t);
  mkdirSync(join(dir, 'bin'));
  const executable = (path, text) => { writeFileSync(path, text); chmodSync(path, 0o755); };
  writeFileSync(join(dir, 'installer.sh'), readFileSync(join(root, 'deploy/install/install.sh'), 'utf8').replaceAll('\r\n', '\n'));
  executable(join(dir, 'hoi-node'), '#!/usr/bin/env bash\necho "hoi-node 1.0.0-preview.1"\n');
  executable(join(dir, 'bin/uname'), '#!/usr/bin/env bash\nif [ "$1" = -s ]; then echo Linux; else echo x86_64; fi\n');
  executable(join(dir, 'bin/id'), '#!/usr/bin/env bash\necho 0\n');
  executable(join(dir, 'bin/mkdir'), '#!/usr/bin/env bash\ncase "$*" in */etc/hoi-node*) exit 0 ;; esac\nexec /bin/mkdir "$@"\n');
  // All privileged mutations are stand-ins; the actual installer only writes its own temp files.
  executable(join(dir, 'bin/install'), '#!/usr/bin/env bash\nprintf "install %s\\n" "$*" >> trace\n');
  executable(join(dir, 'bin/tee'), '#!/usr/bin/env bash\ncat > unit.fixture\nprintf "write %s\\n" "$*" >> trace\n');
  executable(join(dir, 'bin/systemctl'), `#!/usr/bin/env bash
printf 'systemctl %s\\n' "$*" >> trace
case "$1" in
  --version) echo 'systemd 252' ;;
  is-active) exit 1 ;;
  show) echo "\${TEST_STATE:-hoi-public}" ;;
  clean) [ "\${FAIL_CLEAN:-0}" = 0 ] || exit 1 ;;
esac
exit 0
`);
  const run = (role, flags = [], env = {}) => {
    writeFileSync(join(dir, 'trace'), '');
    const result = spawnSync(bash, ['-c', 'export PATH="$PWD/bin:$PATH"; exec bash ./installer.sh "$@"', 'test', role,
      '--binary', './hoi-node', '--yes', '--skip-checks', '--no-firewall', ...flags], { cwd: dir, encoding: 'utf8', env: { ...process.env, ...env } });
    return { ...result, trace: readFileSync(join(dir, 'trace'), 'utf8') };
  };
  const keep = run('public');
  assert.equal(keep.status, 0, keep.stdout + keep.stderr);
  assert.doesNotMatch(keep.trace, /systemctl (stop|clean)/);
  const wrong = run('public', ['--reset-data', '--confirm-reset', 'hoi-node']);
  assert.notEqual(wrong.status, 0);
  assert.match(wrong.stderr, /confirm with --confirm-reset hoi-public/);
  assert.doesNotMatch(wrong.trace, /install |write |systemctl (stop|clean)/);
  const unconfirmed = run('public', ['--reset-data']);
  assert.notEqual(unconfirmed.status, 0);
  assert.doesNotMatch(unconfirmed.trace, /install |write |systemctl (stop|clean)/);
  const check = run('public', ['check', '--reset-data', '--confirm-reset', 'hoi-public']);
  assert.notEqual(check.status, 0);
  assert.match(check.stderr, /not check or config/);
  const reset = run('public', ['--reset-data', '--confirm-reset', 'hoi-public']);
  assert.equal(reset.status, 0, reset.stdout + reset.stderr);
  assert.ok(reset.trace.indexOf('systemctl stop hoi-node') < reset.trace.indexOf('systemctl clean --what=state hoi-node'));
  assert.ok(reset.trace.indexOf('systemctl clean --what=state hoi-node') < reset.trace.indexOf('systemctl restart hoi-node'));
  const mismatch = run('public', ['--reset-data', '--confirm-reset', 'hoi-public'], { TEST_STATE: 'unrelated-data' });
  assert.notEqual(mismatch.status, 0);
  assert.match(mismatch.stderr, /StateDirectory differs/);
  assert.doesNotMatch(mismatch.trace, /systemctl (stop|clean|restart)/);
  const failed = run('private', ['--reset-data', '--confirm-reset', 'hoi-node'], { TEST_STATE: 'hoi-node', FAIL_CLEAN: '1' });
  assert.notEqual(failed.status, 0);
  assert.match(failed.trace, /systemctl clean --what=state hoi-node/);
  assert.doesNotMatch(failed.trace, /systemctl (enable|restart)/);
});
