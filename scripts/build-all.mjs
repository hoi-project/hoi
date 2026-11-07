#!/usr/bin/env node
// Builds everything this computer can build into one folder, a folder per tool, with names that
// say what each file is, a README and SHA256SUMS:
//
//   builds/hoi-<version>/
//     README.md, SHA256SUMS
//     server/       hoi-server_<v>_linux-x64.tar.gz, hoi-node_<v>_x86_64-unknown-linux-gnu.tar.gz
//     node-setup/   hoi-node-setup_<v>_<os>-<arch>-portable.exe|zip, -setup.exe, .dmg, .AppImage, …
//     app/          hoi-desktop_<v>_<os>-<arch>-setup.exe, .dmg, .AppImage, .deb, …
//     website/      hoi-website_<v>.tar.gz
//
// This is for looking at a build, trying it and posting it. The signed, flat release folder that
// install.sh --from and a GitHub release take is scripts/release-local.mjs's (dist/release/).
//
//   pnpm build:all                          everything
//   pnpm build:all --only server,node-setup just these parts
//   pnpm build:all --skip website
//   pnpm build:all --arch all               servers for x64 and arm64
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { basename, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildBundle } from './build-server.mjs';
import {
  LINUX_TARGETS, PARTS, TOOLS, bundleName, describe, hostArch, hostOs, nodeName, portableName, serverName, size, websiteName,
} from './build-names.mjs';
import { mismatches, readVersion } from './release-version.mjs';
import { findBundles } from './tauri-bundles.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
// On Windows pnpm is a .cmd script, which only a shell starts.
const shell = (cmd) => process.platform === 'win32' && cmd === 'pnpm';

const help = `Usage: pnpm build:all [--only PARTS] [--skip PARTS] [--arch x64|arm64|all] [--node-binary PATH] [--out DIR]

Builds into builds/hoi-<version>/, a folder per part, with a README.md and SHA256SUMS.
Parts, in the order they are built:
  server       the Linux server bundle and hoi-node (Docker Desktop, or --node-binary)
  node-setup   hoi Node Setup: installer and a portable app that needs no installing
  app          the hoi app (desktop), installers for this computer
  website      the website and browser app as static files

--only LIST         build just these parts (comma separated)
--skip LIST         build all but these
--arch ARCH         server architecture: x64 (default), arm64 or all
--node-binary PATH  package this Linux hoi-node (or hoi-node .tar.gz) instead of building it in Docker
--out DIR           where the version folders go (default: builds/)

Rebuilding a part replaces only that part's folder; a part that fails leaves the previous one.`;

const HINTS = {
  server: 'Start Docker Desktop (Linux containers), or pass --node-binary PATH with a Linux hoi-node.',
  'node-setup': 'Needs Rust and the Tauri prerequisites for this system (https://tauri.app/start/prerequisites/).',
  app: 'Needs Rust and the Tauri prerequisites for this system (https://tauri.app/start/prerequisites/).',
  website: 'Needs the Rust/WASM prerequisites in docs/DEVELOPMENT.md.',
};

const list = (value) => value.split(',').map((p) => p.trim()).filter(Boolean);

export function parseArgs(argv) {
  const o = { arches: ['x64'], out: join(root, 'builds') };
  let only = null;
  let skip = [];
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    if (flag === '-h' || flag === '--help') return { help: true };
    if (!['--only', '--skip', '--arch', '--node-binary', '--out'].includes(flag)) throw new Error(`unknown option ${flag}`);
    const value = argv[++i];
    if (!value || value.startsWith('--')) throw new Error(`${flag} needs a value`);
    if (flag === '--only') only = list(value);
    else if (flag === '--skip') skip = list(value);
    else if (flag === '--arch') {
      if (!['x64', 'arm64', 'all'].includes(value)) throw new Error('--arch takes x64, arm64 or all');
      o.arches = value === 'all' ? ['x64', 'arm64'] : [value];
    } else if (flag === '--node-binary') o.nodeBinary = resolve(value);
    else o.out = resolve(value);
  }
  for (const part of [...(only ?? []), ...skip]) {
    if (!PARTS.includes(part)) throw new Error(`no part called ${part}: choose from ${PARTS.join(', ')}`);
  }
  o.parts = PARTS.filter((p) => (only ? only.includes(p) : true) && !skip.includes(p));
  if (!o.parts.length) throw new Error('nothing left to build');
  if (o.nodeBinary && o.arches.length > 1) throw new Error('--node-binary is one architecture: give --arch x64 or --arch arm64');
  return o;
}

function run(cmd, args, { cwd = root } = {}) {
  console.log(`$ ${[cmd, ...args].join(' ')}`);
  const r = spawnSync(cmd, args, { cwd, stdio: 'inherit', shell: shell(cmd) });
  if (r.error) throw new Error(`cannot run ${cmd}: ${r.error.message}`);
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(' ')} failed (exit ${r.status})`);
}

function git(args) {
  const r = spawnSync('git', args, { cwd: root, encoding: 'utf8' });
  return r.status === 0 ? r.stdout.trim() : '';
}

/** A part is built into .<folder>.new and swapped in only when it is complete. */
export function stage(dir, folder) {
  if (!Object.values(TOOLS).some((t) => t.folder === folder)) throw new Error(`not a build folder: ${folder}`);
  const target = join(dir, folder);
  const work = join(dir, `.${folder}.new`);
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });
  return {
    work,
    commit() {
      rmSync(target, { recursive: true, force: true });
      renameSync(work, target);
    },
    abandon() {
      rmSync(work, { recursive: true, force: true });
    },
  };
}

function buildServer(o, version, work) {
  for (const arch of o.arches) {
    const tmp = join(work, `.bundle-${arch}`);
    const { directory, archive } = buildBundle({ arch, binary: o.nodeBinary, out: tmp });
    renameSync(archive, join(work, serverName(version, arch)));
    // hoi-node alone, packed as a release archive is: what the node setup app uploads.
    run('tar', ['-C', directory, '-czf', join(work, nodeName(version, LINUX_TARGETS[arch])), 'hoi-node']);
    rmSync(tmp, { recursive: true, force: true });
  }
}

function tauriTarget(tool) {
  return resolve(join(root, 'apps', tool.app, 'src-tauri'), process.env.CARGO_TARGET_DIR ?? 'target');
}

function buildTauri(tool, version, work, install) {
  const target = tauriTarget(tool);
  const bundleDir = join(target, 'release', 'bundle');
  // Bundles of an earlier version would otherwise be copied too.
  for (const f of findBundles(bundleDir)) rmSync(f);
  install();
  run('pnpm', ['--filter', tool.pkg, 'build']);
  const made = findBundles(bundleDir).filter((f) => basename(f).includes(version));
  if (!made.length) throw new Error(`the ${tool.title} build made no bundle named for ${version}`);
  for (const f of made) {
    const name = bundleName(tool.slug, version, basename(f));
    if (name) copyFileSync(f, join(work, name));
  }
  if (tool.binary) portable(tool, version, target, work);
}

/** The app as one file that runs without installing. Linux has its AppImage already. */
function portable(tool, version, target, work) {
  const os = hostOs();
  const out = join(work, portableName(tool.slug, version, os, hostArch()));
  if (os === 'windows') {
    // tauri build leaves the program under mainBinaryName, and Cargo's name may be there too:
    // the one this build wrote is the newer.
    const exe = [`${tool.binary}.exe`, 'hoi-setup.exe'].map((n) => join(target, 'release', n)).filter((p) => existsSync(p))
      .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
    if (!exe) throw new Error(`no ${tool.binary}.exe in ${join(target, 'release')}`);
    copyFileSync(exe, out);
  } else if (os === 'macos') {
    const dir = join(target, 'release', 'bundle', 'macos');
    const app = existsSync(dir) && readdirSync(dir).find((n) => n.endsWith('.app'));
    if (!app) throw new Error(`no .app in ${dir}`);
    run('ditto', ['-c', '-k', '--keepParent', join(dir, app), out]);
  }
}

function buildWebsite(version, work, install) {
  install();
  run('pnpm', ['build']);
  run('tar', ['-C', join(root, 'dist'), '-czf', join(work, websiteName(version)), '.']);
}

/** The files of a version folder, by part, as forward-slash paths relative to it. */
export function listFiles(dir) {
  const files = [];
  for (const part of PARTS) {
    const folder = join(dir, TOOLS[part].folder);
    if (!existsSync(folder)) continue;
    for (const name of readdirSync(folder).sort()) {
      if (name.startsWith('.') || !statSync(join(folder, name)).isFile()) continue;
      files.push({ part, path: `${TOOLS[part].folder}/${name}`, bytes: statSync(join(folder, name)).size });
    }
  }
  return files;
}

export function sha256sums(dir, files) {
  return files.map((f) => `${createHash('sha256').update(readFileSync(join(dir, f.path))).digest('hex')}  ${f.path}\n`).join('');
}

/** The folder's README.md: what was built, from what, and what each file is for. */
export function readme({ version, commit, dirty, date, host, files }) {
  const lines = [
    `# hoi ${version}`,
    '',
    `Built ${date}${commit ? ` from commit \`${commit}\`${dirty ? ' with uncommitted changes' : ''}` : ''} on ${host}, with \`pnpm build:all\`.`,
    'Preview software and not code-signed. Check the files against `SHA256SUMS`: `sha256sum -c SHA256SUMS`,',
    'or `Get-FileHash FILE` in PowerShell.',
    '',
  ];
  const has = (part) => files.some((f) => f.part === part);
  if (has('node-setup') || has('server')) {
    lines.push('## Set up a node on your server', '');
    if (has('node-setup')) {
      lines.push(
        '1. Run **hoi Node Setup** from `node-setup/`. The portable one needs no installing; keep it in this folder.',
        '2. Connect to the server. The wizard finds `server/hoi-node_…` for the server\'s processor, uploads it',
        '   and checks the server before anything changes.',
        '',
      );
    }
    if (has('server')) lines.push('Without the wizard: upload a `server/hoi-server_…` bundle and run its `install.sh` (below).', '');
  }
  for (const part of PARTS) {
    if (!has(part)) continue;
    const tool = TOOLS[part];
    lines.push(`## ${tool.folder}/: ${tool.title}`, '', tool.about, '', '| File | Size | What it is |', '|---|---|---|');
    for (const f of files.filter((x) => x.part === part)) {
      const name = f.path.slice(tool.folder.length + 1);
      lines.push(`| \`${name}\` | ${size(f.bytes)} | ${describe(name).replaceAll('|', '\\|')} |`);
    }
    lines.push('');
  }
  return lines.join('\n');
}

function main() {
  let o;
  try {
    o = parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(`build-all: ${error.message}\n\n${help}`);
    process.exit(2);
  }
  if (o.help) return console.log(help);

  const version = readVersion({ path: 'core/Cargo.toml', kind: 'cargo', section: 'workspace.package' });
  const off = mismatches(version);
  if (off.length) console.warn(`warning: not every package says ${version}: ${off.join(', ')}`);
  const dir = join(o.out, `hoi-${version}`);
  mkdirSync(dir, { recursive: true });

  let installed = false;
  const install = () => {
    if (!installed) run('pnpm', ['install', '--frozen-lockfile']);
    installed = true;
  };
  const builders = {
    server: (work) => buildServer(o, version, work),
    'node-setup': (work) => buildTauri(TOOLS['node-setup'], version, work, install),
    app: (work) => buildTauri(TOOLS.app, version, work, install),
    website: (work) => buildWebsite(version, work, install),
  };

  const failed = [];
  for (const part of o.parts) {
    const tool = TOOLS[part];
    console.log(`\n── ${tool.title} (${tool.folder}/)`);
    const s = stage(dir, tool.folder);
    try {
      builders[part](s.work);
      s.commit();
    } catch (error) {
      s.abandon();
      failed.push(part);
      console.error(`build-all: ${tool.title} was not built: ${error.message}\n  ${HINTS[part]}`);
    }
  }

  const files = listFiles(dir);
  writeFileSync(join(dir, 'SHA256SUMS'), sha256sums(dir, files));
  writeFileSync(join(dir, 'README.md'), readme({
    version,
    commit: git(['rev-parse', '--short', 'HEAD']),
    dirty: Boolean(git(['status', '--porcelain'])),
    date: new Date().toISOString().slice(0, 16).replace('T', ' ') + ' UTC',
    host: `${hostOs()} ${hostArch()}`,
    files,
  }));

  const shown = relative(process.cwd(), dir);
  console.log(`\n── ${shown && !shown.startsWith('..') ? shown : dir}`);
  for (const f of files) console.log(`  ${f.path.padEnd(64)} ${size(f.bytes)}`);
  console.log('  README.md\n  SHA256SUMS');
  if (failed.length) {
    console.error(`\nNot built: ${failed.join(', ')} (see above). The rest is in ${dir}.`);
    process.exitCode = 1;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
