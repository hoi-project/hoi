// The installer (deploy/install/install.sh) against a release folder made here: it must pick
// the hoi-node for this computer from the release list, refuse a file whose SHA-256 differs,
// and write the service each role needs. --dry-run changes nothing on this machine.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { appendFileSync, chmodSync, copyFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const installer = join(root, 'deploy/install/install.sh');
const skip = process.platform === 'win32' ? 'install.ps1 is checked on Windows by the long tests' : false;
const target = `${process.arch === 'arm64' ? 'aarch64' : 'x86_64'}-${process.platform === 'darwin' ? 'apple-darwin' : 'unknown-linux-gnu'}`;

/** A release folder with a stand-in hoi-node for this computer, an app file and its manifest. */
function release() {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-release-'));
  const stage = join(dir, 'stage');
  spawnSync('mkdir', [stage]);
  writeFileSync(join(stage, 'hoi-node'), '#!/bin/sh\necho "hoi-node 9.9.9"\n');
  chmodSync(join(stage, 'hoi-node'), 0o755);
  spawnSync('tar', ['-C', stage, '-czf', join(dir, `hoi-node_9.9.9_${target}.tar.gz`), 'hoi-node']);
  rmSync(stage, { recursive: true });
  writeFileSync(join(dir, 'hoi_9.9.9_amd64.AppImage'), 'not really an AppImage');
  // The node setup app ships beside the app for the same platforms; the installers must not take it for the app.
  writeFileSync(join(dir, 'hoi-node-setup_9.9.9_amd64.AppImage'), 'not really the setup app');
  const manifest = spawnSync('node', [join(root, 'scripts/release-manifest.mjs'), dir, 'v9.9.9'], { encoding: 'utf8' });
  assert.equal(manifest.status, 0, manifest.stderr);
  writeFileSync(join(dir, 'manifest.json'), manifest.stdout);
  return dir;
}

function install(...args) {
  const r = spawnSync('bash', [installer, ...args, ...(args.includes('--from') ? ['--allow-unsigned-local'] : []), '--dry-run', '--yes'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  return { status: r.status, out: r.stdout + r.stderr };
}

/** The installer as it is, without --dry-run: only for runs that stop before changing anything. */
function bare(script, args, env = process.env) {
  const local = args.includes('--from') || existsSync(join(dirname(script), 'manifest.json'));
  const r = spawnSync('bash', [script, ...args, ...(local ? ['--allow-unsigned-local'] : [])], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], env });
  return { status: r.status, out: r.stdout + r.stderr };
}

test('tag downloads use the public repository unless --repo explicitly selects another', { skip }, () => {
  const bin = mkdtempSync(join(tmpdir(), 'hoi-install-gh-'));
  try {
    // Stop at the first fetch: inspect the actual request without network or installation.
    writeFileSync(join(bin, 'gh'), '#!/bin/sh\nprintf "%s\\n" "$*" >&2\nexit 1\n', { mode: 0o755 });
    const env = { ...process.env, PATH: `${bin}:${process.env.PATH}` };
    for (const [args, expected] of [[[], 'hoi-project/hoi'], [['--repo', 'fork/hoi'], 'fork/hoi']]) {
      const result = bare(installer, ['home', '--release', 'v9.9.9', '--dry-run', '--yes', ...args], env);
      assert.notEqual(result.status, 0);
      assert.ok(result.out.includes(`release download v9.9.9 --repo ${expected} --pattern`), result.out);
    }
  } finally {
    rmSync(bin, { recursive: true, force: true });
  }
});

test('a home node gets the hoi-node for this computer, checked, and a service that is a public relay', { skip }, () => {
  const dir = release();
  try {
    const link = 'https://hoi.sh/app/#/node?w=qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq@203.0.113.7:7844';
    // a coded link (the addresses packed out of sight, as nodes write them now) passes the same check
    const coded = 'https://hoi.sh/app/#/node?code=SE9OAV1m6-1etSPtkXN7LS0qm2OTNFkZ_x';
    const { status, out } = install('home', '--from', dir, '--node', link, '--node', coded, '--name', "lena's pi");
    assert.equal(status, 0, out);
    assert.match(out, /hoi 9\.9\.9/);
    assert.match(out, new RegExp(`fetching hoi-node_9\\.9\\.9_${target}\\.tar\\.gz`));
    assert.match(out, /SHA-256 only/, 'says the signature was not checked while no key is published');
    for (const flag of ['--node', '203.0.113.7:7844', "lena's pi", '--peer-bind', 'node-link']) assert.ok(out.includes(flag), `${flag} in\n${out}`);
    assert.ok(out.includes(coded), `the coded link in\n${out}`);
    assert.match(out, /home node and public relay/);
    // Public by default: no flag turns it on, nothing names a registry for it.
    assert.doesNotMatch(out, /--registry|--index|--public-relay|--no-public-relay/);
    assert.match(out, /No registry is needed/);
    // "relay" is the name the same role had before.
    const relay = install('relay', '--from', dir);
    assert.equal(relay.status, 0, relay.out);
    assert.match(relay.out, /home node and public relay/);
    const kept = install('private', '--from', dir);
    assert.equal(kept.status, 0, kept.out);
    assert.ok(kept.out.includes('--no-public-relay'), kept.out);
    assert.doesNotMatch(kept.out, /node-link/);
    assert.match(install('home', '--from', dir, '--node', 'https://hoi.sh/app/#/join?c=x').out, /needs a node link/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a public node carries its operator notice, is warned without one, and refuses a file that is none', { skip }, () => {
  const dir = release();
  try {
    const notice = join(dir, 'hoi-sleepy-walrus.notice.json');
    writeFileSync(notice, JSON.stringify({ v: 1, node: 'sleepy-walrus', operator: 'Lena', contact: 'lena at example dot org', country: 'XX', processes: 'Nothing beyond that.', ipLogging: false, retentionDays: 0, updated: '2026-11-03' }));
    const { status, out } = install('public', '--from', dir, '--notice-file', notice);
    assert.equal(status, 0, out);
    assert.match(out, /ok {5}notice {6}.*by Lena/, 'the check names the operator');
    assert.ok(out.includes(`install -m 0644 ${notice} /etc/hoi-node/notice.json`) || out.includes('/etc/hoi-node/notice.json') || out.includes('Application Support/hoi-node/notice.json'), out);
    assert.match(out, /"--notice" "\/etc\/hoi-node\/notice\.json"|<string>--notice<\/string>/, 'the service passes the notice to the node');
    assert.match(out, /your notice:/);
    // Without one, a public node is installed with a warning that says what to do.
    const bare = install('public', '--from', dir);
    assert.equal(bare.status, 0, bare.out);
    assert.match(bare.out, /warn {3}notice {6}no operator notice/);
    assert.doesNotMatch(bare.out, /"--notice"|<string>--notice</);
    // A home node needs none and is not warned.
    assert.doesNotMatch(install('home', '--from', dir).out, /notice/);
    // Who and where are optional: a notice without them installs, and the check says what people read.
    const anonymous = join(dir, 'anonymous.notice.json');
    writeFileSync(anonymous, JSON.stringify({ v: 1, node: 'sleepy-walrus', processes: 'Nothing beyond that.', ipLogging: false, retentionDays: 0, updated: '2026-11-03' }));
    const unnamed = install('public', '--from', dir, '--notice-file', anonymous);
    assert.equal(unnamed.status, 0, unnamed.out);
    assert.match(unnamed.out, /ok {5}notice {6}.*names no operator: people who connect read that you chose not to say who you are/);
    // A file that is no notice stops before anything: no "processes" in it.
    const junk = join(dir, 'junk.json');
    writeFileSync(junk, '{"hello":"there"}');
    const bad = install('public', '--from', dir, '--notice-file', junk);
    assert.notEqual(bad.status, 0);
    assert.match(bad.out, /is no notice: no "processes"/);
    assert.match(install('public', '--from', dir, '--notice-file', join(dir, 'missing.json')).out, /no such notice file/);
    assert.match(install('desktop', '--from', dir, '--notice-file', notice).out, /goes with a node/);
    // The configuration a kit carries names the notice next to it, and the install finds it there.
    const made = install('config', 'public', '--name', 'sleepy walrus', '--out', join(dir, 'hoi-sleepy-walrus.conf'), '--notice-file', notice);
    assert.equal(made.status, 0, made.out);
    assert.match(made.out, /notice_file=hoi-sleepy-walrus\.notice\.json/);
    writeFileSync(join(dir, 'hoi-sleepy-walrus.conf'), 'role=public\nname=sleepy walrus\nhostname=sleepy-walrus\nnotice_file=hoi-sleepy-walrus.notice.json\n');
    const fromConf = install('--config', join(dir, 'hoi-sleepy-walrus.conf'), '--from', dir);
    assert.equal(fromConf.status, 0, fromConf.out);
    assert.match(fromConf.out, /ok {5}notice {6}.*by Lena/);
    assert.match(fromConf.out, /"--notice"|<string>--notice</);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a registry announces to itself, allows the apps, and keeps its data apart from a person key', { skip }, () => {
  const dir = release();
  try {
    assert.match(install('registry', '--from', dir).out, /needs --registry-url/);
    const { status, out } = install('registry', '--from', dir, '--registry-url', 'https://names.example.org');
    assert.equal(status, 0, out);
    for (const flag of ['--public', '--registry', '127.0.0.1:7845', '--registry-url', 'tauri://localhost', '--port-mapping']) assert.ok(out.includes(flag), `${flag} in\n${out}`);
    assert.match(out, /--index"?\s*\n?\s*(<string>)?"?https:\/\/names\.example\.org/);
    assert.match(out, /hoi-public|\/public</);
    assert.match(out, /caddy reverse-proxy --from names\.example\.org/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the release list marks the node setup app, and the installer leaves it out of the app choice', { skip }, () => {
  const dir = release();
  try {
    const files = JSON.parse(readFileSync(join(dir, 'manifest.json'), 'utf8')).releases[0].files;
    assert.deepEqual(files.filter((f) => f.product === 'setup').map((f) => f.name), ['hoi-node-setup_9.9.9_amd64.AppImage']);
    assert.equal(files.find((f) => f.name === 'hoi_9.9.9_amd64.AppImage').product, undefined);
    // The functions the installer reads the list with, on this manifest.
    const functions = readFileSync(installer, 'utf8').match(/^entries\(\) \{[^]*?^\}\n[^]*?^pick\(\) \{[^]*?^\}/m)[0];
    const r = spawnSync('bash', ['-c', `work=${JSON.stringify(dir)}\n${functions}\nentries | cut -d" " -f4; echo --; pick linux AppImage x86_64 | cut -d" " -f4`], { encoding: 'utf8' });
    assert.equal(r.status, 0, r.stderr);
    assert.doesNotMatch(r.stdout, /hoi-node-setup/);
    assert.match(r.stdout, /--\nhoi_9\.9\.9_amd64\.AppImage\n$/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a file that differs from the release list is not installed', { skip }, () => {
  const dir = release();
  try {
    appendFileSync(join(dir, `hoi-node_9.9.9_${target}.tar.gz`), 'x');
    const { status, out } = install('public', '--from', dir);
    assert.notEqual(status, 0);
    assert.match(out, /does not match the release list's SHA-256: not installing it/);
    assert.doesNotMatch(out, /\+ (install|write)/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('config writes a node for a server under a random name, and the server installs from it', { skip }, () => {
  const dir = release();
  const here = mkdtempSync(join(tmpdir(), 'hoi-config-'));
  const make = (...args) => {
    const r = spawnSync('bash', [installer, 'config', ...args, '--yes'], { cwd: here, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    return { status: r.status, out: r.stdout + r.stderr };
  };
  try {
    assert.match(make('registry').out, /a registry needs --domain/);
    assert.match(make('public', '--domain', 'example.org').out, /--domain is for a registry/);
    // What the file cannot carry is refused, not dropped: it is given on the server.
    assert.match(make('--node', 'https://hoi.sh/app/#/node?w=q@203.0.113.7:7844').out, /give other flags on the server/);
    const dry = make('--dry-run');
    assert.equal(dry.status, 0, dry.out);
    assert.match(dry.out, /\+ write hoi-[a-z]+-[a-z]+-\d{5}\.conf:\n# A hoi public node/);
    assert.deepEqual(readdirSync(here), [], 'a dry run writes nothing');
    const made = make('registry', '--domain', 'Example.ORG');
    assert.equal(made.status, 0, made.out);
    const [, file, host] = /wrote (hoi-([a-z]+-[a-z]+-\d{5})\.conf)/.exec(made.out) ?? [];
    assert.ok(file, made.out);
    assert.match(made.out, /Nothing on this computer changed/);
    const text = readFileSync(join(here, file), 'utf8');
    for (const line of ['role=registry', `name=${host}`, `hostname=${host}`, `registry_url=https://${host}.example.org`]) {
      assert.match(text, new RegExp(`^${line.replace(/[.]/g, '\\.')}$`, 'm'), text);
    }
    // Another run, another name; a file that is there already stays as it was.
    const other = make();
    assert.equal(other.status, 0, other.out);
    assert.match(other.out, /a hoi public node called [a-z]+-[a-z]+-\d{5}/);
    assert.ok(!other.out.includes(host), other.out);
    assert.match(make('registry', '--domain', 'example.org', '--out', file).out, /already there: not overwriting it/);
    assert.equal(readFileSync(join(here, file), 'utf8'), text);

    // On the server: the file's role, name and registry, its host name for the computer, and
    // flags given there win over the file.
    const conf = join(here, file);
    const { status, out } = install('--config', conf, '--from', dir);
    assert.equal(status, 0, out);
    for (const flag of ['--public', '--registry', `https://${host}.example.org`, `"${host}"`]) assert.ok(out.includes(flag), `${flag} in\n${out}`);
    if (process.platform === 'linux') assert.match(out, new RegExp(`hostnamectl set-hostname ${host}\\n`));
    else assert.match(out, /a Mac keeps the name it has/);
    const own = install('--config', conf, '--from', dir, '--name', 'my own label');
    assert.equal(own.status, 0, own.out);
    assert.match(own.out, /--name"?\s*(<\/string>\s*<string>)?"?my own label/);
    assert.ok(!own.out.includes(`"${host}"`), own.out);

    // The file is read, never run: what is not a known key=value line is refused.
    const bad = (body) => {
      writeFileSync(join(here, 'bad.conf'), body);
      return install('--config', join(here, 'bad.conf'), '--from', dir);
    };
    const quiet = bad(`role=public\nname=$(touch ${join(here, 'pwned')})\nhostname=quiet-otter-12345\n`);
    assert.equal(quiet.status, 0, quiet.out);
    assert.ok(!existsSync(join(here, 'pwned')), 'nothing in the file runs');
    assert.match(bad('role=public\nrm -rf /\n').out, /not a key=value line/);
    assert.match(bad('role=public\nshell=1\n').out, /no such setting: shell/);
    assert.match(bad('role=home\n').out, /role is public or registry/);
    assert.match(bad('role=public\nhostname=Bad_Host\n').out, /a host name takes lowercase letters/);
    assert.match(install('desktop', '--config', conf).out, /--config describes a node/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
    rmSync(here, { recursive: true, force: true });
  }
});

test('nothing happens without a role, or with an address that is not https', { skip }, () => {
  assert.match(install().out, /say what to install/);
  assert.match(install('home', '--index', 'http://registry.example').out, /needs an https:\/\/ registry URL/);
  assert.match(install('public', '--extra', '--name $(reboot)').out, /plain flags/);
});

test('check looks at the computer, one line each, and changes nothing', { skip }, () => {
  const dir = release();
  try {
    const { status, out } = bare(installer, ['check', 'public', '--from', dir, '--name', 'quiet-otter-12345']);
    assert.match(out, /^Checking this computer for a hoi public node called quiet-otter-12345:$/m);
    for (const what of ['system', 'source', 'disk', 'memory']) assert.match(out, new RegExp(`^  (ok|info|warn|BLOCK) +${what} `, 'm'), `${what} in\n${out}`);
    assert.match(out, /^  ok +source +hoi 9\.9\.9, hoi-node_9\.9\.9_/m);
    assert.match(out, /SHA-256 only/);
    // Whether this computer could take it depends on it (systemd, ports); the answer is one of two.
    assert.match(out, status === 0 ? /^Ready to install/m : /^Not ready: \d+ things? above \(BLOCK\)/m);
    assert.equal(status === 0, !/^  BLOCK/m.test(out), out);
    assert.doesNotMatch(out, /^\+ /m, 'a check runs nothing that changes the computer');
    assert.match(install('check', 'desktop').out, /check looks at this computer for a node/);
    assert.match(install('config', 'check').out, /one at a time/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a BLOCK stops the install before anything changes; a dry run goes on to show it', { skip }, () => {
  // A hoi-node that isn't there is a BLOCK on every computer.
  const stopped = bare(installer, ['public', '--binary', '/nonexistent/hoi-node', '--yes']);
  assert.equal(stopped.status, 2, stopped.out);
  assert.match(stopped.out, /^  BLOCK +source +no such file: \/nonexistent\/hoi-node$/m);
  assert.match(stopped.out, /nothing was changed: fix what says BLOCK above/);
  assert.doesNotMatch(stopped.out, /fetching|runs as a service|runs in your account|systemctl|launchctl/);
  const checked = bare(installer, ['check', 'public', '--binary', '/nonexistent/hoi-node']);
  assert.equal(checked.status, 1, checked.out);
  const dry = install('public', '--binary', '/nonexistent/hoi-node');
  assert.match(dry.out, /a dry run goes on to show what the install would do/);
});

test('a registry gets Caddy for its address with --tls caddy, and only a registry with a host name', { skip }, () => {
  const dir = release();
  try {
    assert.match(install('public', '--tls', 'caddy', '--from', dir).out, /goes with registry/);
    assert.match(install('registry', '--tls', 'nginx', '--registry-url', 'https://names.example.org').out, /--tls takes caddy/);
    assert.match(install('registry', '--tls', 'caddy', '--registry-url', 'https://203.0.113.10').out, /needs a domain name/);
    assert.match(install('registry', '--tls', 'caddy', '--registry-url', 'https://example.org/registry').out, /root of its own host name/);
    const { status, out } = install('registry', '--tls', 'caddy', '--registry-url', 'https://names.example.org', '--hostname', 'quiet-otter-12345', '--from', dir);
    assert.equal(status, 0, out);
    assert.match(out, /^Checking this computer for a hoi public node and registry called quiet-otter-12345:$/m);
    assert.match(out, /^  (ok|warn|BLOCK) +https /m);
    assert.match(out, /^  (ok|warn) +dns +names\.example\.org/m);
    if (process.platform === 'linux') {
      assert.match(out, /\+ write \/etc\/caddy\/hoi-registry\.caddy:\n# Written by deploy\/install\/install\.sh \(registry\)[^\n]*\nnames\.example\.org \{\n\treverse_proxy 127\.0\.0\.1:7845\n\}/);
      assert.match(out, /reload-or-restart caddy/);
      assert.match(out, /hostnamectl set-hostname quiet-otter-12345\n/);
      assert.match(out, /Caddy serves it at https:\/\/names\.example\.org/);
    }
    assert.doesNotMatch(out, /caddy reverse-proxy --from/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('config carries --tls caddy; the server reads it back', { skip }, () => {
  const here = mkdtempSync(join(tmpdir(), 'hoi-config-'));
  try {
    const r = spawnSync('bash', [installer, 'config', 'registry', '--domain', 'example.org', '--tls', 'caddy', '--yes'], { cwd: here, encoding: 'utf8' });
    assert.equal(r.status, 0, r.stderr);
    const [file] = readdirSync(here);
    const text = readFileSync(join(here, file), 'utf8');
    assert.match(text, /^tls=caddy$/m);
    assert.match(r.stdout, /check --config hoi-[a-z]+-[a-z]+-\d{5}\.conf/);
    assert.match(install('--config', join(here, file), '--binary', '/nonexistent').out, /^  (ok|warn|BLOCK) +https /m);
    assert.match(spawnSync('bash', [installer, 'config', '--no-firewall', '--yes'], { cwd: here, encoding: 'utf8' }).stderr, /give other flags on the server/);
  } finally {
    rmSync(here, { recursive: true, force: true });
  }
});

test('the release files beside the installer are used as they are, offline', { skip }, () => {
  const dir = release();
  try {
    copyFileSync(installer, join(dir, 'install.sh'));
    const beside = spawnSync('bash', [join(dir, 'install.sh'), 'public', '--dry-run', '--yes', '--allow-unsigned-local'], { encoding: 'utf8' });
    assert.equal(beside.status, 0, beside.stderr);
    assert.match(beside.stderr, /installing from the release files beside the installer/);
    assert.match(beside.stdout, /^  ok +source +hoi 9\.9\.9, hoi-node_9\.9\.9_[^ ]+ from /m);
    // A source named on the command line wins over the files beside it.
    const named = spawnSync('bash', [join(dir, 'install.sh'), 'public', '--binary', '/nonexistent', '--dry-run', '--yes'], { encoding: 'utf8' });
    assert.doesNotMatch(named.stderr, /beside the installer/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('an active firewall is opened for the node, unless --no-firewall', { skip: skip || (process.getuid?.() !== 0 && 'reads a stand-in ufw, which only root runs without sudo resetting PATH') }, () => {
  const dir = release();
  const bin = mkdtempSync(join(tmpdir(), 'hoi-bin-'));
  try {
    writeFileSync(join(bin, 'ufw'), '#!/bin/sh\n[ "$1" = status ] && printf "Status: active\\n\\nTo Action From\\n7843/udp                   ALLOW       Anywhere\\n"\nexit 0\n');
    chmodSync(join(bin, 'ufw'), 0o755);
    const env = { ...process.env, PATH: `${bin}:${process.env.PATH}` };
    const opened = bare(installer, ['public', '--from', dir, '--dry-run', '--yes'], env);
    assert.match(opened.out, /^  info +firewall +ufw is on: the install opens UDP 7844, 7846, 7847 in it$/m);
    assert.match(opened.out, /^\+ (sudo )?ufw allow 7844\/udp comment hoi-node$/m);
    assert.match(opened.out, /^\+ (sudo )?ufw allow 7846\/udp comment hoi-node$/m);
    assert.doesNotMatch(opened.out, /ufw allow 7843/);
    const left = bare(installer, ['public', '--from', dir, '--no-firewall', '--dry-run', '--yes'], env);
    assert.match(left.out, /^  warn +firewall +ufw is on and doesn't let UDP 7844, 7846, 7847 in/m);
    assert.doesNotMatch(left.out, /ufw allow/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
    rmSync(bin, { recursive: true, force: true });
  }
});

test('a public node keeps mailboxes for private communities, 2 GB unless --hold-pieces-gb says otherwise, and config carries it', { skip }, () => {
  const dir = release();
  const here = mkdtempSync(join(tmpdir(), 'hoi-config-'));
  try {
    const on = install('public', '--from', dir, '--hold-pieces-gb', '2');
    assert.equal(on.status, 0, on.out);
    assert.match(on.out, /"--hold-pieces" "2"|<string>--hold-pieces<\/string>/, 'the service passes it to the node');
    assert.match(on.out, /(ok|warn) {4,5}pieces {6}.*encrypted pieces and (private communities' )?mailboxes/);
    // off when not given, or 0
    // on by default for a public node (S6), and 0 turns it off where the node would hold by default
    assert.match(install('public', '--from', dir).out, /"--hold-pieces" "2"|<string>--hold-pieces<\/string>/, 'on by default');
    assert.match(install('public', '--from', dir, '--hold-pieces-gb', '0').out, /"--hold-pieces" "0"|<string>0<\/string>/, '0 is passed on');
    // only numbers in range, and only for a node strangers' devices reach
    assert.match(install('public', '--from', dir, '--hold-pieces-gb', 'lots').out, /from 0\.1 to 10000/);
    assert.match(install('public', '--from', dir, '--hold-pieces-gb', '0.01').out, /from 0\.1 to 10000/);
    assert.match(install('home', '--from', dir, '--hold-pieces-gb', '2').out, /for a public node or a registry/);
    const r = spawnSync('bash', [installer, 'config', 'public', '--hold-pieces-gb', '3', '--yes'], { cwd: here, encoding: 'utf8' });
    assert.equal(r.status, 0, r.stderr);
    const [file] = readdirSync(here);
    assert.match(readFileSync(join(here, file), 'utf8'), /^hold_pieces_gb=3$/m);
    assert.match(install('--config', join(here, file), '--from', dir).out, /"--hold-pieces" "3"|<string>--hold-pieces<\/string>/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
    rmSync(here, { recursive: true, force: true });
  }
});

test('release installation fails closed and rejects a signed rollback before installing', { skip: process.platform !== 'linux' }, () => {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-signed-installer-'));
  try {
    const fixture = JSON.parse(readFileSync(join(root, 'scripts/fixtures/release-signed.json'), 'utf8'));
    const manifest = JSON.parse(fixture.manifest);
    const file = manifest.releases[0].files[0];
    writeFileSync(join(dir, file.name), fixture.artifact);
    writeFileSync(join(dir, 'manifest.json'), fixture.manifest);
    writeFileSync(join(dir, 'manifest.json.minisig'), fixture.signature);
    // Exercise the shell's verifier contract with the independently cross-checked JS verifier.
    // No OS installer or test artifact is executed (--dry-run).
    const verifier = join(dir, 'minisign');
    writeFileSync(verifier, `#!${process.execPath}\n(async () => {\nconst { readFileSync } = await import('node:fs');\nconst { verifyMinisign } = await import(${JSON.stringify(new URL('./release-verify.mjs', import.meta.url).href)});\nconst args = process.argv.slice(2), value = flag => args[args.indexOf(flag) + 1];\nverifyMinisign(readFileSync(value('-Vqm')), readFileSync(value('-x'), 'utf8'), value('-P'));\n})().catch(() => process.exit(1));\n`);
    chmodSync(verifier, 0o755);
    const script = join(dir, 'install.sh'), stamp = join(dir, 'release-version');
    writeFileSync(script, readFileSync(installer, 'utf8').replace('release_stamp="$HOME/.local/share/hoi/release-version"', `release_stamp='${stamp}'`));
    const run = (...extra) => {
      const r = spawnSync('bash', [script, 'desktop', '--from', dir, '--dry-run', '--yes', ...extra], { encoding: 'utf8', env: { ...process.env, PATH: `${dir}:${process.env.PATH}` } });
      return { code: r.status, text: r.stdout + r.stderr };
    };
    const unconfigured = run();
    assert.notEqual(unconfigured.code, 0); assert.match(unconfigured.text, /signing key is not configured/);
    const valid = run('--pubkey', fixture.key);
    assert.equal(valid.code, 0, valid.text); assert.match(valid.text, /signature good/);
    writeFileSync(stamp, '2.0.1\n');
    const rollback = run('--pubkey', fixture.key);
    assert.notEqual(rollback.code, 0); assert.match(rollback.text, /refusing to downgrade/);
    rmSync(stamp);
    appendFileSync(join(dir, 'manifest.json'), ' ');
    const changed = run('--pubkey', fixture.key);
    assert.notEqual(changed.code, 0); assert.match(changed.text, /signature does not verify/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
