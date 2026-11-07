// The real installer (deploy/install/install.sh), for the server kit's tests: its word lists, a
// stand-in release folder like scripts/install.test.mjs makes, a kit unpacked with the system's
// tar, and installer runs that change nothing (--dry-run, or check).
import { spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync, copyFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '../../../../..');
export const installerPath = join(root, 'deploy/install/install.sh');
export const unix = process.platform !== 'win32';
/** Linux writes a systemd unit and sets the host name; a Mac a launchd agent and keeps its own. */
export const linux = process.platform === 'linux';
const target = `${process.arch === 'arm64' ? 'aarch64' : 'x86_64'}-${process.platform === 'darwin' ? 'apple-darwin' : 'unknown-linux-gnu'}`;

export const readInstaller = () => readFileSync(installerPath, 'utf8');

/** The words of one bash array in install.sh: NAME=(a b c …). */
export function installerWords(name) {
  const m = new RegExp(`^${name}=\\(([^)]*)\\)`, 'm').exec(readInstaller());
  if (!m) throw new Error(`no ${name} in install.sh`);
  return m[1].split(/\s+/).filter(Boolean);
}

const dirs = [];
export function tempDir(prefix = 'hoi-kit-') {
  const d = mkdtempSync(join(tmpdir(), prefix));
  dirs.push(d);
  return d;
}
export function cleanup() {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
}

/** A release folder with a stand-in hoi-node for this computer and its manifest. */
export function release() {
  const dir = tempDir('hoi-release-');
  const stage = join(dir, 'stage');
  mkdirSync(stage);
  writeFileSync(join(stage, 'hoi-node'), '#!/bin/sh\necho "hoi-node 9.9.9"\n');
  chmodSync(join(stage, 'hoi-node'), 0o755);
  const archive = spawnSync('tar', ['-C', stage, '-czf', join(dir, `hoi-node_9.9.9_${target}.tar.gz`), 'hoi-node']);
  if (archive.error) throw archive.error;
  if (archive.status !== 0) throw new Error(String(archive.stderr));
  rmSync(stage, { recursive: true });
  const manifest = spawnSync('node', [join(root, 'scripts/release-manifest.mjs'), dir, 'v9.9.9'], { encoding: 'utf8' });
  if (manifest.error) throw manifest.error;
  if (manifest.status !== 0) throw new Error(manifest.stderr);
  if (!JSON.parse(manifest.stdout).releases?.length) throw new Error('The release fixture generated no release.');
  writeFileSync(join(dir, 'manifest.json'), manifest.stdout);
  return dir;
}

/** Copies a release folder's files into `dest`, as someone putting them beside the installer. */
export function copyRelease(from, dest) {
  for (const n of readdirSync(from)) copyFileSync(join(from, n), join(dest, n));
}

/** Unpacks an archive with the system's tar; returns the folder it unpacked into and what tar listed. */
export function unpack(name, bytes) {
  const dir = tempDir();
  const file = join(dir, name);
  writeFileSync(file, bytes);
  const list = spawnSync('tar', ['-tvf', file], { encoding: 'utf8' });
  const x = spawnSync('tar', ['-xf', file, '-C', dir], { encoding: 'utf8' });
  if (x.status !== 0) throw new Error(x.stderr);
  return { dir, listing: list.stdout, list: (spawnSync('tar', ['-tf', file], { encoding: 'utf8' }).stdout ?? '').split('\n').filter(Boolean) };
}

export const read = (path) => readFileSync(path, 'utf8');
export const isExecutable = (path) => (statSync(path).mode & 0o111) !== 0;

/** Runs an installer; with `dry`, --dry-run --yes, which change nothing on this computer. */
export function run(script, args, { cwd, dry = true } = {}) {
  const local = args.includes('--from') || existsSync(join(dirname(script), 'manifest.json'));
  // Fictitious registry hosts must not depend on the machine's DNS timeout.
  const bin = tempDir('hoi-kit-tools-');
  writeFileSync(join(bin, 'getent'), '#!/bin/sh\nexit 2\n', { mode: 0o755 });
  const r = spawnSync('bash', [script, ...args, ...(local ? ['--allow-unsigned-local'] : []), ...(dry ? ['--dry-run', '--yes'] : [])], { cwd, env: { ...process.env, PATH: `${bin}:${process.env.PATH ?? ''}` }, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  return { status: r.status, out: (r.stdout ?? '') + (r.stderr ?? '') };
}

/** What `config` writes for these flags, in a folder of its own. */
export function configWith(args) {
  const here = tempDir('hoi-config-');
  const r = run(installerPath, ['config', ...args], { cwd: here, dry: false });
  const file = /wrote (\S+\.conf)/.exec(r.out)?.[1];
  return { ...r, text: file ? readFileSync(join(here, file), 'utf8') : '' };
}

/** A file in a temporary folder of its own, for a test that hands the installer a path. */
export function writeTemp(name, text) {
  const dir = tempDir('hoi-file-');
  const path = join(dir, name);
  writeFileSync(path, text);
  return path;
}
