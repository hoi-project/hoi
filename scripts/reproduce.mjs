// Reproducible builds: the parts of a release that come out byte for byte the
// same on any machine with the same toolchains, and their SHA-256.
//
//   node scripts/reproduce.mjs [--only wasm,node,static] [--target <rust triple>]
//       builds each part, prints { commit, epoch, parts: { name: sha256 } } as JSON
//   node scripts/reproduce.mjs --twice [--only …]
//       builds everything twice into fresh directories and fails on any difference
//   node scripts/reproduce.mjs --compare <digests.json> [--only …]
//       builds once and compares with digests written earlier, a release's for instance
//
// What makes the bytes the same: the toolchain in core/rust-toolchain.toml, `--locked` Cargo
// and pnpm lockfiles, every build path remapped (scripts/private-build-paths.mjs), and
// SOURCE_DATE_EPOCH set to the commit's time. The parts:
//   wasm    apps/web/src/lib/core/wasm/pkg/hoi_wasm_bg.wasm (wasm-bindgen and wasm-opt included)
//   node    the hoi-node release binary for --target (default: this machine's)
//   static  every file of the assembled static site and app (dist/), hashed as one tree
// The desktop bundles (installers signed per platform, with their own timestamps) are not
// byte for byte: their programs are built the same way, their packaging is not.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { privateBuildPaths } from './private-build-paths.mjs';

const root = resolve(import.meta.dirname, '..');
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const value = (name) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : undefined; };
const only = (value('--only') ?? 'wasm,node,static').split(',').filter(Boolean);
for (const part of only) if (!['wasm', 'node', 'static'].includes(part)) throw new Error(`unknown part ${part}`);
const target = value('--target');

const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');

/** Every file under `dir`, sorted, as `path sha256\n`, hashed again: one digest for a tree. */
export function treeDigest(dir) {
  const files = [];
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.isFile()) files.push(p);
    }
  };
  walk(dir);
  const lines = files.map((p) => `${relative(dir, p).split('\\').join('/')} ${sha256(readFileSync(p))}`).sort();
  return { digest: sha256(lines.join('\n') + '\n'), files: lines.length };
}

function commitTime() {
  try { return execFileSync('git', ['log', '-1', '--format=%ct'], { cwd: root, encoding: 'utf8' }).trim(); }
  catch { return '0'; }
}

/** The environment every build runs in: fixed paths, the commit's time, nothing of this machine. */
export function reproducibleEnv(base = process.env, epoch = commitTime(), targetDir) {
  const env = privateBuildPaths(root, { ...base, SOURCE_DATE_EPOCH: epoch, TZ: 'UTC', LC_ALL: 'C', CARGO_INCREMENTAL: '0' });
  if (targetDir) env.CARGO_TARGET_DIR = targetDir;
  return env;
}

function build(part, env) {
  const run = (cmd, a, cwd = root) => execFileSync(cmd, a, { cwd, env, stdio: ['ignore', 'inherit', 'inherit'] });
  if (part === 'wasm') {
    run('node', ['scripts/build-wasm.mjs']);
    return { sha256: sha256(readFileSync(resolve(root, 'apps/web/src/lib/core/wasm/pkg/hoi_wasm_bg.wasm'))) };
  }
  if (part === 'node') {
    const t = target ? ['--target', target] : [];
    run('cargo', ['build', '--release', '--locked', '-p', 'hoi-node', ...t], resolve(root, 'core'));
    const dir = env.CARGO_TARGET_DIR ?? resolve(root, 'core/target');
    const exe = target?.includes('windows') ? 'hoi-node.exe' : 'hoi-node';
    return { sha256: sha256(readFileSync(resolve(dir, target ?? '', 'release', exe))) };
  }
  // the site and app, assembled as `pnpm build` does; the app carries the wasm part
  run('pnpm', ['tokens']);
  run('pnpm', ['build:site']);
  run('pnpm', ['build:web']);
  run('node', ['scripts/assemble-static.mjs']);
  const { digest, files } = treeDigest(resolve(root, 'dist'));
  return { sha256: digest, files };
}

function buildAll(targetDir) {
  const env = reproducibleEnv(process.env, commitTime(), targetDir);
  const parts = {};
  for (const part of ['wasm', 'node', 'static'].filter((p) => only.includes(p) || (p === 'wasm' && only.includes('static')))) {
    parts[part] = build(part, env);
  }
  for (const p of Object.keys(parts)) if (!only.includes(p)) delete parts[p];
  return { commit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(), epoch: env.SOURCE_DATE_EPOCH, target: target ?? null, parts };
}

function differences(a, b) {
  return Object.keys({ ...a.parts, ...b.parts }).filter((k) => a.parts[k]?.sha256 !== b.parts[k]?.sha256);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (flag('--twice')) {
    const dirs = ['a', 'b'].map((x) => resolve(root, `core/target/reproduce-${x}`));
    const runs = dirs.map((dir) => {
      rmSync(dir, { recursive: true, force: true });
      mkdirSync(dir, { recursive: true });
      return buildAll(dir);
    });
    const diff = differences(runs[0], runs[1]);
    console.log(JSON.stringify({ first: runs[0], second: runs[1], same: diff.length === 0 }, null, 2));
    if (diff.length) {
      console.error(`not reproducible: ${diff.join(', ')}`);
      process.exit(1);
    }
  } else if (value('--compare')) {
    const theirs = JSON.parse(readFileSync(value('--compare'), 'utf8'));
    const ours = buildAll();
    const diff = differences(ours, { parts: Object.fromEntries(Object.entries(theirs.parts ?? {}).filter(([k]) => only.includes(k))) });
    console.log(JSON.stringify({ ours, theirs, same: diff.length === 0 }, null, 2));
    if (diff.length) {
      console.error(`differs from the digests given: ${diff.join(', ')}`);
      process.exit(1);
    }
  } else {
    console.log(JSON.stringify(buildAll(), null, 2));
  }
}
