#!/usr/bin/env node
// Builds a hoi release on your own computer, without GitHub Actions: the same checks, hoi-node
// archives, the desktop bundle for this system and the minisign-signed manifest that
// .github/workflows/release.yml makes, with the installers beside them, in dist/release/v<version>/
// (a folder install.sh installs from as it is, offline too). Optionally tags the
// version, pushes the tag and uploads everything to a draft GitHub release.
//
//   node scripts/release-local.mjs 0.2.0 --bump              set 0.2.0 everywhere, commit, check, build
//   node scripts/release-local.mjs 0.2.0                     check and build what the tree says is 0.2.0
//   node scripts/release-local.mjs 0.2.0 --quick --no-desktop   hoi-node only, quick checks
//   node scripts/release-local.mjs 0.2.0 --docker --targets x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu
//   node scripts/release-local.mjs 0.2.0 --tag --push --upload   then publish it as a draft release
//   node scripts/release-local.mjs --check-version 0.2.0     exit 1 unless every package says 0.2.0
//
// Options:
//   --bump              write <version> into every Cargo.toml, tauri.conf.json and package.json that
//                       carries one, refresh the lockfiles, and commit (from a clean tree)
//   --quick             only quick checks for a local build (not full release validation)
//   --no-audit          skip only the dependency audits (cargo deny, audit, vet; pnpm audit), which
//                       need the network for their advisory databases
//   --long              major versions only; browser suites already run by default for majors
//   --no-desktop        skip the desktop bundle
//   --targets LIST      hoi-node targets, comma separated (default: this computer's); a target
//                       other than this computer's needs its Rust target and a linker for it
//   --docker            build the Linux targets in Docker instead (buildx; native on a matching
//                       CPU, emulated otherwise, so slow); the binaries need glibc 2.36 or newer
//   --minisign-key PATH the secret key that signs manifest.json (default: $MINISIGN_KEY, else
//                       ~/.minisign/minisign.key); without one the manifest stays unsigned
//   --tag               create the annotated tag v<version> at HEAD
//   --push              push that tag to origin; this starts the release workflow when the
//                       HOI_RELEASES_ENABLED is "true" (hoi-project/hoi)
//   --upload            upload the files to the draft release v<version> (gh; the tag must be on
//                       GitHub already, so with --push or after it), made if it doesn't exist
//   --allow-dirty       build from a tree with uncommitted changes (never with --tag or --upload)
import { spawnSync, spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { configureInstaller, loadPublication } from './publication.mjs';
import { checkCandidate } from './release-check.mjs';
import { releaseTier } from './release-tier.mjs';
import { findBundles, releaseBundleName } from './tauri-bundles.mjs';
import { checkVersion, planVersion, applyVersion } from './release-version.mjs';
export { LOCKFILES, lockWithVersion, mismatches, readVersion, versionFiles, writeVersion } from './release-version.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
// On Windows pnpm is a .cmd script, which only a shell starts; everything else is an .exe and
// starts without one, so arguments with spaces (release notes) arrive intact.
const shell = (cmd) => process.platform === 'win32' && cmd === 'pnpm';

function usage(message) {
  if (message) console.error(`release-local: ${message}`);
  console.error('usage: node scripts/release-local.mjs <version> [--bump] [--quick] [--no-audit] [--long] [--no-desktop] [--no-setup] [--targets LIST] [--docker]\n' +
    '         [--minisign-key PATH] [--tag] [--push] [--upload] [--allow-dirty]\n' +
    '       node scripts/release-local.mjs --check-version <version>');
  process.exit(2);
}

function parse(argv) {
  const o = { targets: [], desktop: true, setup: true, audit: true };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const value = () => argv[++i] ?? usage(`${a} needs a value`);
    switch (a) {
      case '--check-version': o.checkOnly = true; o.version = value(); break;
      case '--bump': o.bump = true; break;
      case '--quick': o.quick = true; break;
      case '--no-audit': o.audit = false; break;
      case '--long': o.long = true; break;
      case '--no-desktop': o.desktop = false; break;
      case '--no-setup': o.setup = false; break;
      case '--targets': o.targets = value().split(',').map((t) => t.trim()).filter(Boolean); break;
      case '--docker': o.docker = true; break;
      case '--minisign-key': o.key = value(); break;
      case '--tag': o.tag = true; break;
      case '--push': o.push = true; break;
      case '--upload': o.upload = true; break;
      case '--allow-dirty': o.dirty = true; break;
      case '-h':
      case '--help':
        return usage();
      default:
        if (a.startsWith('-') || o.version) usage(`unexpected ${a}`);
        o.version = a.replace(/^v/, '');
    }
  }
  try { o.tier = releaseTier(o.version ?? ''); } catch { usage('a version like 0.2.0 is needed'); }
  if (o.long && o.tier !== 'major') usage('--long is restricted to major versions (X.0.0)');
  if (o.long && o.quick) usage('--long and --quick cannot be combined');
  return o;
}

function step(title) {
  console.log(`\n── ${title}`);
}

/** Runs a command in the repository (or `cwd`), stopping the release when it fails. */
function sh(cmd, args, { cwd = root, env, allowFail = false, capture = false } = {}) {
  console.log(`$ ${[cmd, ...args].join(' ')}`);
  const r = spawnSync(cmd, args, { cwd, env: { ...process.env, ...env }, stdio: capture ? ['ignore', 'pipe', 'inherit'] : 'inherit', shell: shell(cmd), encoding: 'utf8' });
  if (r.status !== 0 && !allowFail) {
    console.error(`release-local: ${cmd} ${args.join(' ')} failed${r.error ? `: ${r.error.message}` : ''}`);
    process.exit(1);
  }
  return r;
}

function has(cmd, args = ['--version']) {
  return spawnSync(cmd, args, { stdio: 'ignore', shell: shell(cmd) }).status === 0;
}

function git(args) {
  return spawnSync('git', args, { cwd: root, encoding: 'utf8' }).stdout.trim();
}

export function repoSlug(env = process.env) {
  // Local releases may be built in the private maintainer checkout. Its origin is
  // not the public download destination; forks can explicitly select their own.
  return env.GITHUB_REPOSITORY || 'hoi-project/hoi';
}

function hostTarget() {
  const out = spawnSync('rustc', ['-vV'], { cwd: join(root, 'core'), encoding: 'utf8' }).stdout ?? '';
  const host = out.match(/^host: (\S+)$/m)?.[1];
  if (!host) {
    console.error('release-local: rustc is needed (https://rustup.rs)');
    process.exit(1);
  }
  return host;
}

function bump(version) {
  step(`Setting version ${version}`);
  if (git(['status', '--porcelain'])) usage('--bump needs a clean tree, so the version change is a commit of its own');
  applyVersion(planVersion(version));
  checkVersion(version);
  sh('git', ['commit', '--quiet', '-am', `chore(release): v${version}`]);
}

function checks(o) {
  step(`Release checks: ${o.quick ? 'quick' : o.tier}`);
  sh('node', ['scripts/check-quick.mjs']);
  if (o.quick || o.tier === 'patch') return;
  if (o.tier === 'minor') {
    sh('pnpm', ['install', '--frozen-lockfile']);
    sh('pnpm', ['--filter', '@hoi/site', 'check']);
    sh('pnpm', ['--filter', '@hoi/core-api', 'check']);
    sh('pnpm', ['--filter', '@hoi/web', 'test']);
    sh('cargo', ['fmt', '--all', '--check'], { cwd: join(root, 'core') });
    sh('cargo', ['test', '--locked', '-p', 'hoi-types', '-p', 'hoi-trust', '--lib'], { cwd: join(root, 'core') });
    return;
  }
  step('Core: format, clippy, tests');
  const core = join(root, 'core');
  sh('cargo', ['fmt', '--all', '--check'], { cwd: core });
  sh('cargo', ['clippy', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings'], { cwd: core });
  sh('cargo', ['test', '--locked', '--workspace'], { cwd: core });
  step('Web: tokens, wasm core, type checks, tests');
  sh('pnpm', ['install', '--frozen-lockfile']);
  sh('pnpm', ['--filter', '@hoi/tokens', 'check']);
  sh('pnpm', ['core:wasm']);
  sh('pnpm', ['check']);
  sh('pnpm', ['test']);
  sh('pnpm', ['test:sandbox']);
  if (o.audit) audits();
  long();
}

function audits() {
  step('Dependency audits');
  sh('node', ['scripts/check-dependency-security.mjs']);
  const missing = [];
  for (const manifest of ['core/Cargo.toml', 'apps/desktop/src-tauri/Cargo.toml', 'apps/setup/src-tauri/Cargo.toml']) {
    if (has('cargo', ['deny', '--version'])) sh('cargo', ['deny', '--manifest-path', manifest, '--all-features', 'check']);
    else missing.push('cargo-deny');
    if (has('cargo', ['audit', '--version'])) sh('cargo', ['audit', '--file', join(dirname(manifest), 'Cargo.lock')]);
    else missing.push('cargo-audit');
  }
  if (has('cargo', ['vet', '--version'])) sh('cargo', ['vet', '--locked'], { cwd: join(root, 'core') });
  else missing.push('cargo-vet');
  sh('pnpm', ['audit', '--audit-level', 'low']);
  for (const tool of new Set(missing)) console.warn(`warning: ${tool} is not installed, so its check was skipped (cargo install ${tool} --locked)`);
}

/** The browser suites of the long tests, with the relay Worker they need running beside them. */
function long() {
  step('Browser end-to-end suites');
  sh('pnpm', ['build:site']);
  sh('pnpm', ['build:web']);
  sh('node', ['scripts/assemble-static.mjs']);
  sh('pnpm', ['exec', 'playwright', 'install', 'chromium']);
  for (const s of ['scripts/e2e-vault.mjs', 'apps/site/scripts/e2e.mjs --quick', 'scripts/e2e-discovery.mjs', 'scripts/e2e-registry-browser.mjs']) {
    const [file, ...rest] = s.split(' ');
    sh('node', [file, ...rest]);
  }
  mkdirSync(join(root, 'test-results'), { recursive: true });
  sh('node', ['scripts/e2e-deployment.mjs']);
  const relay = spawn('pnpm', ['dlx', 'wrangler@4', 'dev', '--config', 'relay/wrangler.toml', '--local', '--ip', '127.0.0.1', '--port', '8788'], { cwd: root, stdio: 'ignore', shell: shell('pnpm') });
  try {
    sh('node', ['scripts/e2e-private-relay.mjs'], { env: { HOI_PRIVATE_RELAY_TEST_URL: 'http://127.0.0.1:8788' } });
  } finally {
    relay.kill();
  }
  sh('cargo', ['build', '--locked', '--manifest-path', 'core/Cargo.toml', '-p', 'hoi-node']);
  for (const s of ['e2e-peer-only', 'e2e-registry-node', 'e2e-network', 'e2e-rtc', 'e2e-public-network']) sh('node', [`scripts/${s}.mjs`]);
}

/** Packs one hoi-node binary the way the release workflow does. */
function pack(binary, version, target, out) {
  const name = basename(binary);
  const archive = join(out, `hoi-node_${version}_${target}.tar.gz`);
  sh('tar', ['-C', dirname(binary), '-czf', archive, name]);
  return archive;
}

function buildNode(o, out) {
  const host = hostTarget();
  const targets = o.targets.length ? o.targets : [host];
  for (const target of targets) {
    const linux = target.endsWith('-unknown-linux-gnu');
    if (o.docker && linux) {
      const platform = { 'x86_64-unknown-linux-gnu': 'linux/amd64', 'aarch64-unknown-linux-gnu': 'linux/arm64' }[target];
      if (!platform) usage(`--docker builds x86_64 and aarch64 Linux, not ${target}`);
      step(`hoi-node for ${target} in Docker (${platform})`);
      const dest = join(out, '.docker', target);
      sh('docker', ['buildx', 'build', '--platform', platform, '-f', 'core/crates/hoi-node/Dockerfile', '--target', 'binary', '--output', `type=local,dest=${dest}`, '.']);
      pack(join(dest, 'hoi-node'), o.version, target, out);
      continue;
    }
    step(`hoi-node for ${target}`);
    if (target !== host && has('rustup')) sh('rustup', ['target', 'add', target], { cwd: join(root, 'core') });
    sh('cargo', ['build', '--release', '--locked', '-p', 'hoi-node', '--target', target], { cwd: join(root, 'core') });
    const exe = target.includes('windows') ? 'hoi-node.exe' : 'hoi-node';
    // Cargo puts it under CARGO_TARGET_DIR when that is set (relative to where cargo ran).
    const targetDir = resolve(join(root, 'core'), process.env.CARGO_TARGET_DIR ?? 'target');
    pack(join(targetDir, target, 'release', exe), o.version, target, out);
  }
  rmSync(join(out, '.docker'), { recursive: true, force: true });
}

/** One Tauri app's bundles for this system: the desktop app or the node setup app. */
function buildTauri(o, out, { app, name, label }) {
  step(`${label} bundle for this system`);
  const target = resolve(join(root, 'apps', app, 'src-tauri'), process.env.CARGO_TARGET_DIR ?? 'target');
  const bundleDir = join(target, 'release', 'bundle');
  const before = new Set(findBundles(bundleDir));
  for (const f of before) rmSync(f);
  sh('pnpm', ['install', '--frozen-lockfile']);
  sh('pnpm', ['--filter', name, 'build']);
  const made = findBundles(bundleDir).filter((f) => basename(f).includes(o.version) || /\.app\.tar\.gz(\.sig)?$/.test(f));
  if (!made.length) {
    console.error(`release-local: the ${label.toLowerCase()} build made no bundle named for this version`);
    process.exit(1);
  }
  for (const f of made) {
    const arch = f.includes('universal-apple-darwin') ? 'universal' : process.arch === 'arm64' ? 'aarch64' : 'x86_64';
    const name = basename(f).replace(/\.app\.tar\.gz(\.sig)?$/, `_${o.version}_${arch}.app.tar.gz$1`);
    copyFileSync(f, join(out, releaseBundleName(name)));
  }
}

const buildDesktop = (o, out) => buildTauri(o, out, { app: 'desktop', name: '@hoi/desktop', label: 'Desktop' });
const buildSetup = (o, out) => buildTauri(o, out, { app: 'setup', name: '@hoi/setup', label: 'Node setup app' });

/** The installers go into the release as well, so its folder, or the files downloaded from the
 * GitHub release, installs as it is: install.sh uses the release files beside it. */
export const INSTALLERS = ['deploy/install/install.sh', 'deploy/install/install.ps1'];
function installers(out) {
  for (const path of INSTALLERS) writeFileSync(join(out, basename(path)), configureInstaller(readFileSync(join(root, path), 'utf8')));
}

function signingKey(o) {
  const key = o.key ?? process.env.MINISIGN_KEY ?? join(homedir(), '.minisign', 'minisign.key');
  return existsSync(key) ? key : null;
}

function manifest(o, out) {
  step('Manifest');
  const r = sh('node', ['scripts/release-manifest.mjs', out, `v${o.version}`], { capture: true, env: { GITHUB_REPOSITORY: repoSlug() } });
  writeFileSync(join(out, 'manifest.json'), r.stdout);
  const key = signingKey(o);
  if (!key) {
    console.warn('warning: no minisign key (--minisign-key, $MINISIGN_KEY or ~/.minisign/minisign.key): manifest.json is NOT signed');
    return false;
  }
  if (!has('minisign', ['-v'])) {
    console.warn('warning: minisign is not installed (https://jedisct1.github.io/minisign/): manifest.json is NOT signed');
    return false;
  }
  sh('minisign', ['-S', '-s', key, '-m', join(out, 'manifest.json'), '-t', `hoi v${o.version}`]);
  const trustedKey = loadPublication().manifestKey;
  if (trustedKey) sh('minisign', ['-Vm', join(out, 'manifest.json'), '-P', trustedKey]);
  return true;
}

function tagAndPush(o) {
  const tag = `v${o.version}`;
  if (o.tag) {
    step(`Tag ${tag}`);
    if (git(['tag', '--list', tag])) console.log(`${tag} exists already; left as it is`);
    else sh('git', ['tag', '-a', tag, '-m', `hoi ${tag}`]);
  }
  if (o.push) {
    step(`Push ${tag}`);
    sh('git', ['push', 'origin', tag]);
    console.log('If HOI_RELEASES_ENABLED is "true" (hoi-project/hoi), the release workflow starts now and builds into the same draft release.');
  }
}

/** Whether the draft release exists; when it does, what other computers put there joins `out`. */
function fetchDraft(o, out) {
  const tag = `v${o.version}`;
  const env = { ...process.env, GH_REPO: repoSlug() };
  if (spawnSync('gh', ['release', 'view', tag], { stdio: 'ignore', env }).status !== 0) return false;
  step(`Files already in the draft release ${tag}`);
  // The manifest is made again below, over this computer's files and those.
  sh('gh', ['release', 'download', tag, '--dir', out, '--skip-existing'], { env: { GH_REPO: env.GH_REPO } });
  for (const n of ['manifest.json', 'manifest.json.minisig']) rmSync(join(out, n), { force: true });
  return true;
}

function upload(o, out, signed, exists) {
  const tag = `v${o.version}`;
  step(`Draft release ${tag}`);
  loadPublication(true);
  if (!signed) usage('refusing to upload an unsigned manifest; give the configured minisign signing key');
  const env = { GH_REPO: repoSlug() };
  if (!exists) {
    const notes = `Built on a computer with scripts/release-local.mjs. Desktop installers are not code-signed unless stated. Verify downloads against the signed manifest.json (minisign).`;
    const { prerelease } = checkVersion(o.version);
    sh('gh', ['release', 'create', tag, '--draft', '--verify-tag', `--prerelease=${prerelease}`, '--title', `hoi ${tag}`, '--notes', notes], { env });
  }
  const files = readdirSync(out).filter((n) => statSync(join(out, n)).isFile()).map((n) => join(out, n));
  sh('gh', ['release', 'upload', tag, ...files, '--clobber'], { env });
  console.log(`Uploaded ${files.length} files to the draft release ${tag}; publish it on GitHub when ready.`);
}

function main() {
  const o = parse(process.argv.slice(2));
  if (o.upload || o.push) { loadPublication(true); process.env.HOI_PUBLICATION = '1'; }
  if (o.checkOnly) {
    checkVersion(o.version);
    console.log(`Every package says ${o.version}.`);
    return;
  }
  if ((o.tag || o.upload) && o.dirty) usage('--allow-dirty cannot be combined with --tag or --upload');
  // Before a long build, not after it: what the upload at the end will need.
  if (o.upload && !has('gh')) usage('--upload needs the GitHub CLI (gh), signed in');
  if (o.upload && !(signingKey(o) && has('minisign', ['-v']))) usage('--upload signs the manifest: install minisign and give the configured key (--minisign-key)');
  if (o.bump) bump(o.version);
  checkVersion(o.version);
  if (!o.dirty && git(['status', '--porcelain'])) usage('uncommitted changes: commit them, or add --allow-dirty for a test build');
  checks(o);
  const out = join(root, 'dist', 'release', `v${o.version}`);
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  buildNode(o, out);
  if (o.desktop) buildDesktop(o, out);
  if (o.setup) buildSetup(o, out);
  tagAndPush(o);
  // Before the draft's files: this checkout's installers, not an older upload's.
  installers(out);
  const exists = o.upload && fetchDraft(o, out);
  const signed = manifest(o, out);
  if (signed) checkCandidate(out);
  if (o.upload) upload(o, out, signed, exists);
  step('Done');
  for (const n of readdirSync(out)) console.log(`  ${relative(root, join(out, n))}`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
