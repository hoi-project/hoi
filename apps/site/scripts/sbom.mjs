#!/usr/bin/env node
// The software bill of materials behind /wiki/sbom/ and its CycloneDX download.
//
//   node apps/site/scripts/sbom.mjs          regenerate apps/site/wiki/sbom/sbom.json
//                                            (needs cargo and an installed workspace: it runs
//                                            `cargo metadata --filter-platform` per program and
//                                            reads licenses from node_modules)
//   node apps/site/scripts/sbom.mjs --check  verify sbom.json still matches every lockfile
//                                            (no cargo, no network; part of check:quick)
//
// Programs are dependency closures: what a shipped binary or bundle is actually built from,
// per target platform, with build-only and test-only packages marked. The lockfiles are the
// source of truth for names, versions and checksums; sbom.json adds licenses, descriptions
// and repositories, and is rejected when a lockfile changes without it.
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
const OUT = join(repo, 'apps/site/wiki/sbom/sbom.json');

const LINUX = ['x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu'];
const DESKTOP = ['x86_64-unknown-linux-gnu', 'aarch64-apple-darwin', 'x86_64-apple-darwin', 'x86_64-pc-windows-msvc'];

/** Every program the repository builds, and where its dependencies come from. */
export const PROGRAMS = [
  { id: 'hoi-node', title: 'hoi-node', ships: true, wiki: '/wiki/hoi-node/', summary: 'The headless home and public node: the Linux server bundle and the Docker image.', cargo: { lock: 'core', member: 'hoi-node', targets: LINUX } },
  { id: 'browser-core', title: 'Browser core (hoi-wasm)', ships: true, wiki: '/wiki/web-app/', summary: 'The Rust core compiled to WebAssembly: everything the web app does with keys, records and sync runs in it.', cargo: { lock: 'core', member: 'hoi-wasm', targets: ['wasm32-unknown-unknown'] } },
  { id: 'web-app', title: 'Web app', ships: true, wiki: '/wiki/web-app/', summary: 'The Svelte browser app. Runtime packages are bundled into the app; development packages only build and test it.', npm: ['apps/web', 'packages/core-api', 'packages/tokens'] },
  { id: 'website', title: 'Website (hoi.sh)', ships: true, wiki: '/wiki/website/', summary: 'This static site, including this wiki and the network playground.', npm: ['apps/site', 'packages/tokens', 'packages/vite-plugin-hardening'] },
  { id: 'desktop', title: 'Desktop companion', ships: true, wiki: '/wiki/desktop/', summary: 'The Tauri app for macOS, Windows and Linux, with the core (hoi-core, hoi-net) compiled in natively. Its interface is the web app.', cargo: { lock: 'apps/desktop/src-tauri', member: 'hoi-desktop', targets: DESKTOP }, npm: ['apps/desktop'] },
  { id: 'node-setup', title: 'hoi Node Setup', ships: true, wiki: '/wiki/node-setup/', summary: 'The Tauri wizard that installs and manages a node over SSH.', cargo: { lock: 'apps/setup/src-tauri', member: 'hoi-setup', targets: DESKTOP }, npm: ['apps/setup'] },
  { id: 'mobile-core', title: 'Mobile core (hoi-ffi)', ships: true, wiki: '/wiki/mobile/', summary: 'The Rust core behind the Swift and Kotlin bindings, for Android and iOS devices.', cargo: { lock: 'core', member: 'hoi-ffi', targets: ['aarch64-linux-android', 'aarch64-apple-ios'] } },
  { id: 'android', title: 'Android library', ships: true, wiki: '/wiki/mobile/', summary: 'The Kotlin wrapper module and its Gradle build.', gradle: true },
  { id: 'ios', title: 'iOS package', ships: true, wiki: '/wiki/mobile/', summary: 'The Swift package: the generated bindings, a small wrapper and the theme. It declares no third-party Swift packages.', swift: true },
  { id: 'hoi-sim', title: 'Simulator (hoi-sim)', ships: false, wiki: '/wiki/simulator/', summary: 'The network and moderation simulator, run by developers.', cargo: { lock: 'core', member: 'hoi-sim', targets: LINUX } },
  { id: 'core-tests', title: 'Core tests', ships: false, wiki: '/wiki/development/', summary: 'Packages in the core lockfile that only the test suites use (property tests, test servers and helpers).', cargo: { lock: 'core', rest: true } },
  { id: 'fuzz', title: 'Fuzzing', ships: false, wiki: '/wiki/development/', summary: 'The cargo-fuzz targets\' own workspace.', cargo: { lock: 'core/fuzz', member: 'hoi-fuzz', targets: ['x86_64-unknown-linux-gnu'] } },
  { id: 'tooling', title: 'Repository tooling', ships: false, wiki: '/wiki/development/', summary: 'Browser tests, accessibility checks and the WebAssembly optimizer.', npm: ['.'] },
  { id: 'vendored', title: 'Vendored patches', ships: true, wiki: '/wiki/supply-chain/', summary: 'Published packages kept in vendor/ with small reviewed patches; they replace the registry versions in the builds above.', vendor: true },
  { id: 'ci', title: 'Build services and toolchains', ships: false, wiki: '/wiki/development/', summary: 'Toolchains, container images, GitHub Actions (pinned by commit) and tools CI installs.', ci: true },
];

/* ---------- small parsers ---------- */

/** Cargo.lock: [[package]] tables. */
export function parseCargoLock(text) {
  const out = [];
  for (const block of text.split(/^\[\[package\]\]\s*$/m).slice(1)) {
    const get = (k) => (block.match(new RegExp(`^${k} = "([^"]*)"`, 'm')) || [])[1];
    const deps = (block.match(/^dependencies = \[([\s\S]*?)\]/m) || [, ''])[1].match(/"[^"]+"/g)?.map((s) => s.slice(1, -1)) ?? [];
    out.push({ name: get('name'), version: get('version'), source: get('source') ?? null, checksum: get('checksum') ?? null, deps });
  }
  return out;
}

/** Just enough YAML for pnpm-lock.yaml (v9): block mappings, scalars, flow values kept as text. */
export function parseYaml(text) {
  const root = {};
  const stack = [{ indent: -1, obj: root }];
  for (const raw of text.split('\n')) {
    if (!raw.trim() || raw.trimStart().startsWith('#')) continue;
    const indent = raw.length - raw.trimStart().length;
    const line = raw.trim();
    const m = /^('(?:[^']|'')*'|"[^"]*"|[^:]+?):(?:\s+(.*))?$/.exec(line);
    if (!m) continue;
    let key = m[1];
    if (key.startsWith("'")) key = key.slice(1, -1).replace(/''/g, "'");
    else if (key.startsWith('"')) key = key.slice(1, -1);
    while (stack[stack.length - 1].indent >= indent) stack.pop();
    const parent = stack[stack.length - 1].obj;
    if (m[2] === undefined || m[2] === '') {
      const obj = {};
      parent[key] = obj;
      stack.push({ indent, obj });
    } else {
      let v = m[2];
      if (/^'.*'$/.test(v)) v = v.slice(1, -1).replace(/''/g, "'");
      parent[key] = v;
    }
  }
  return root;
}

const npmKey = (name, version) => `npm:${name}@${version}`;
const cargoKey = (name, version) => `cargo:${name}@${version}`;
const baseVersion = (v) => String(v).replace(/\(.*$/, '');
const splitAt = (spec) => { const i = spec.lastIndexOf('@'); return [spec.slice(0, i), spec.slice(i + 1)]; };

/* ---------- reading the lockfiles (no cargo needed) ---------- */

export function lockfiles() {
  const cargo = {};
  for (const lock of ['core', 'apps/desktop/src-tauri', 'apps/setup/src-tauri', 'core/fuzz']) {
    cargo[lock] = parseCargoLock(readFileSync(join(repo, lock, 'Cargo.lock'), 'utf8'));
  }
  const pnpm = parseYaml(readFileSync(join(repo, 'pnpm-lock.yaml'), 'utf8'));
  return { cargo, pnpm };
}

/** Each npm importer's closure: { key → 'prod' | 'dev' } plus which keys are direct. */
function npmClosure(pnpm, importer) {
  const imp = pnpm.importers[importer];
  if (!imp) throw new Error(`pnpm-lock.yaml has no importer ${importer}`);
  const kinds = new Map();
  const direct = new Set();
  const snap = pnpm.snapshots ?? {};
  const visit = (name, ver, kind) => {
    if (String(ver).startsWith('link:') || String(ver).startsWith('file:')) return;
    const full = `${name}@${ver}`;
    const k = npmKey(name, baseVersion(ver));
    const had = kinds.get(k);
    if (had === 'prod' || had === kind) return;
    kinds.set(k, kind);
    const s = snap[full] ?? snap[`${name}@${baseVersion(ver)}`];
    if (!s || typeof s !== 'object') return;
    for (const field of ['dependencies', 'optionalDependencies']) {
      for (const [n, v] of Object.entries(s[field] ?? {})) visit(n, v, kind);
    }
  };
  for (const [field, kind] of [['dependencies', 'prod'], ['optionalDependencies', 'prod'], ['devDependencies', 'dev']]) {
    for (const [n, spec] of Object.entries(imp[field] ?? {})) {
      const v = spec.version;
      if (String(v).startsWith('link:') || String(v).startsWith('file:')) continue;
      direct.add(npmKey(n, baseVersion(v)));
      visit(n, v, kind);
    }
  }
  return { kinds, direct };
}

/** Every npm package in the lockfile, with its integrity and platform limits. */
function npmPackages(pnpm) {
  const out = new Map();
  for (const [spec, meta] of Object.entries(pnpm.packages ?? {})) {
    const [name, version] = splitAt(spec);
    const integ = /integrity: ([^,}]+)/.exec(meta.resolution ?? '')?.[1] ?? null;
    out.set(npmKey(name, version), { eco: 'npm', name, version, integrity: integ, os: meta.os ?? null, cpu: meta.cpu ?? null });
  }
  return out;
}

/* ---------- other ecosystems (read straight from the build files) ---------- */

function gradle() {
  const toml = readFileSync(join(repo, 'apps/mobile/android/gradle/libs.versions.toml'), 'utf8');
  const versions = Object.fromEntries([...toml.split('[libraries]')[0].matchAll(/^(\w+) = "([^"]+)"/gm)].map((m) => [m[1], m[2]]));
  const bom = versions.composeBom;
  const out = [];
  const lib = toml.split('[libraries]')[1].split('[plugins]')[0];
  for (const m of lib.matchAll(/^[\w-]+ = \{ module = "([^"]+)"(?:, version\.ref = "(\w+)")? \}/gm)) {
    const [group, artifact] = m[1].split(':');
    out.push({ eco: 'maven', name: m[1], group, artifact, version: m[2] ? versions[m[2]] : `BOM ${bom}`, bom: !m[2] });
  }
  for (const m of toml.split('[plugins]')[1].matchAll(/^[\w-]+ = \{ id = "([^"]+)", version\.ref = "(\w+)" \}/gm)) {
    out.push({ eco: 'gradle-plugin', name: m[1], version: versions[m[2]] });
  }
  const wrapper = /gradle-([\d.]+)-bin\.zip/.exec(readFileSync(join(repo, 'apps/mobile/android/gradle/wrapper/gradle-wrapper.properties'), 'utf8'));
  if (wrapper) out.push({ eco: 'tool', name: 'Gradle', version: wrapper[1], url: `https://services.gradle.org/distributions/gradle-${wrapper[1]}-bin.zip` });
  return out;
}

function swift() {
  const pkg = readFileSync(join(repo, 'apps/mobile/ios/HoiCore/Package.swift'), 'utf8');
  const tools = /swift-tools-version:\s*([\d.]+)/.exec(pkg)?.[1];
  const deps = [...pkg.matchAll(/\.package\(\s*url:\s*"([^"]+)"[^)]*\)/g)].map((m) => ({ eco: 'swift', name: m[1], version: '' }));
  return [{ eco: 'tool', name: 'Swift tools', version: tools ?? '', url: 'https://www.swift.org/' }, ...deps];
}

function vendored() {
  const out = [];
  for (const dir of readdirSync(join(repo, 'vendor'))) {
    const p = join(repo, 'vendor', dir);
    if (existsSync(join(p, 'Cargo.toml'))) {
      const t = readFileSync(join(p, 'Cargo.toml'), 'utf8');
      const pkg = t.split(/^\[package\]/m)[1] ?? '';
      const name = /^name = "([^"]+)"/m.exec(pkg)?.[1] ?? dir;
      const version = /^version = "([^"]+)"/m.exec(pkg)?.[1] ?? '';
      const license = /^license = "([^"]+)"/m.exec(pkg)?.[1] ?? '';
      const description = /^description = "([^"]+)"/m.exec(pkg)?.[1] ?? '';
      out.push({ eco: 'cargo', name, version, license, description, path: `vendor/${dir}`, upstream: `https://crates.io/crates/${name}/${version}` });
    } else if (existsSync(join(p, 'package.json'))) {
      const j = JSON.parse(readFileSync(join(p, 'package.json'), 'utf8'));
      out.push({ eco: 'npm', name: j.name, version: j.version, license: j.license ?? '', description: j.description ?? '', path: `vendor/${dir}`, upstream: `https://www.npmjs.com/package/${j.name}/v/${j.version}` });
    }
  }
  return out;
}

function ci() {
  const out = [];
  const seen = new Set();
  const wf = join(repo, '.github/workflows');
  for (const f of readdirSync(wf).filter((f) => f.endsWith('.yml')).sort()) {
    const t = readFileSync(join(wf, f), 'utf8');
    for (const m of t.matchAll(/uses: ([\w.-]+\/[\w./-]+)@([0-9a-f]{40})(?:\s*#\s*(\S+))?/g)) {
      const k = `${m[1]}@${m[2]}`;
      if (seen.has(k)) continue;
      seen.add(k);
      const [owner, name] = m[1].split('/');
      out.push({ eco: 'github-action', name: m[1], version: m[3] ?? m[2].slice(0, 12), sha: m[2], url: `https://github.com/${owner}/${name}/tree/${m[2]}` });
    }
    for (const m of t.matchAll(/cargo install ([\w-]+)(?: --locked)?(?: --version ([\w.-]+))?/g)) {
      const k = `cargo-install:${m[1]}`;
      if (seen.has(k)) continue;
      seen.add(k);
      out.push({ eco: 'cargo-tool', name: m[1], version: m[2] ?? 'latest', url: `https://crates.io/crates/${m[1]}${m[2] ? '/' + m[2] : ''}` });
    }
  }
  const toolchain = /channel = "([^"]+)"/.exec(readFileSync(join(repo, 'core/rust-toolchain.toml'), 'utf8'))?.[1];
  out.push({ eco: 'tool', name: 'Rust toolchain', version: toolchain, url: `https://releases.rs/docs/${toolchain}/` });
  const node = readFileSync(join(repo, '.node-version'), 'utf8').trim();
  out.push({ eco: 'tool', name: 'Node.js', version: node, url: `https://nodejs.org/en/download/archive/v${node}` });
  const pm = JSON.parse(readFileSync(join(repo, 'package.json'), 'utf8')).packageManager;
  if (pm) out.push({ eco: 'tool', name: 'pnpm', version: pm.split('@')[1], url: `https://github.com/pnpm/pnpm/releases/tag/v${pm.split('@')[1]}` });
  for (const df of ['core/crates/hoi-node/Dockerfile', 'registry/Dockerfile', 'sandbox/Dockerfile', 'sandbox/App.Dockerfile', 'sandbox/Control.Dockerfile']) {
    if (!existsSync(join(repo, df))) continue;
    for (const m of readFileSync(join(repo, df), 'utf8').matchAll(/^FROM ([\w./:-]+?)(?:@(sha256:[0-9a-f]{64}))?(?:\s+AS\s+\w+)?\s*$/gim)) {
      if (m[1] === 'scratch') continue;
      const k = `docker:${m[1]}@${m[2] ?? ''}`;
      if (seen.has(k)) continue;
      seen.add(k);
      const [image, tag] = m[1].split(':');
      out.push({ eco: 'docker', name: image, version: tag ?? 'latest', digest: m[2] ?? null, file: df, url: `https://hub.docker.com/_/${image}` });
    }
  }
  return out;
}

/* ---------- regenerating (needs cargo and node_modules) ---------- */

function cargoMetadata(lock, target) {
  const r = spawnSync('cargo', ['metadata', '--format-version', '1', '--locked', '--manifest-path', join(repo, lock, 'Cargo.toml'), ...(target ? ['--filter-platform', target] : [])], { encoding: 'utf8', maxBuffer: 1 << 30 });
  if (r.status !== 0) throw new Error(`cargo metadata ${lock} ${target ?? ''}: ${r.stderr}`);
  return JSON.parse(r.stdout);
}

/** Closure of a workspace member over normal and build edges: { pkgId → 'run' | 'build' }. */
function cargoClosure(meta, memberName) {
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const member = meta.packages.find((p) => p.name === memberName && meta.workspace_members.includes(p.id));
  if (!member) throw new Error(`no workspace member ${memberName}`);
  const kinds = new Map();
  const direct = new Set();
  const visit = (id, kind, depth) => {
    const had = kinds.get(id);
    if (had === 'run' || had === kind) return;
    kinds.set(id, kind);
    for (const d of nodes.get(id)?.deps ?? []) {
      const ks = d.dep_kinds.map((k) => k.kind);
      if (ks.every((k) => k === 'dev')) continue;
      const k = kind === 'build' || ks.every((x) => x === 'build') ? 'build' : 'run';
      if (depth === 0) direct.add(d.pkg);
      visit(d.pkg, k, depth + 1);
    }
  };
  visit(member.id, 'run', 0);
  kinds.delete(member.id);
  return { kinds, direct };
}

function npmMeta(name, version) {
  const store = join(repo, 'node_modules/.pnpm');
  const dirName = `${name.replace('/', '+')}@${version}`;
  if (!existsSync(store)) return null;
  const dir = readdirSync(store).find((d) => d === dirName || d.startsWith(dirName + '_'));
  const file = dir && join(store, dir, 'node_modules', name, 'package.json');
  let j;
  if (file && existsSync(file)) j = JSON.parse(readFileSync(file, 'utf8'));
  else {
    // Not installed on this platform (an optional native binary): ask the registry once.
    const r = spawnSync('curl', ['-fsS', '--max-time', '20', `https://registry.npmjs.org/${name.replace('/', '%2F')}/${version}`], { encoding: 'utf8' });
    if (r.status !== 0) return null;
    j = JSON.parse(r.stdout);
  }
  const repoUrl = typeof j.repository === 'string' ? j.repository : j.repository?.url;
  return {
    license: typeof j.license === 'string' ? j.license : j.license?.type ?? (Array.isArray(j.licenses) ? j.licenses.map((l) => l.type).join(' OR ') : ''),
    description: j.description ?? '',
    repository: normalizeRepo(repoUrl) ?? j.homepage ?? null,
  };
}

function normalizeRepo(u) {
  if (!u) return null;
  let s = String(u).replace(/^git\+/, '').replace(/\.git$/, '').replace(/^git:\/\//, 'https://').replace(/^ssh:\/\/git@github\.com/, 'https://github.com');
  if (/^github:/.test(s)) s = 'https://github.com/' + s.slice(7);
  if (/^[\w.-]+\/[\w.-]+$/.test(s)) s = 'https://github.com/' + s;
  return /^https?:\/\//.test(s) ? s : null;
}

function regenerate() {
  const { cargo, pnpm } = lockfiles();
  const packages = {};
  const programs = [];
  const clean = (s) => (s ?? '').replace(/\s+/g, ' ').trim();

  // Cargo: package metadata from an unfiltered run per lockfile, closures from filtered runs.
  const full = {};
  for (const lock of Object.keys(cargo)) {
    full[lock] = cargoMetadata(lock);
    for (const p of full[lock].packages) {
      if (full[lock].workspace_members.includes(p.id)) continue;
      const vendoredPath = p.source ? null : relative(repo, dirname(p.manifest_path));
      packages[cargoKey(p.name, p.version)] ??= {
        eco: 'cargo', name: p.name, version: p.version, license: clean(p.license) || (p.license_file ? 'see license file' : ''),
        description: clean(p.description), repository: normalizeRepo(p.repository) ?? p.homepage ?? null,
        checksum: cargo[lock].find((x) => x.name === p.name && x.version === p.version)?.checksum ?? null,
        vendored: vendoredPath && vendoredPath.startsWith('vendor/') ? vendoredPath : null,
        local: vendoredPath && !vendoredPath.startsWith('vendor/') ? vendoredPath : null,
      };
    }
  }
  const reached = { core: new Set() };
  for (const prog of PROGRAMS) {
    const entry = { id: prog.id, cargo: [], npm: [], other: [] };
    if (prog.cargo && !prog.cargo.rest) {
      const merged = new Map();
      const direct = new Set();
      for (const target of prog.cargo.targets) {
        const meta = cargoMetadata(prog.cargo.lock, target);
        const byId = new Map(meta.packages.map((p) => [p.id, p]));
        const c = cargoClosure(meta, prog.cargo.member);
        for (const [id, kind] of c.kinds) {
          const p = byId.get(id);
          if (meta.workspace_members.includes(id)) continue;
          const k = cargoKey(p.name, p.version);
          const prev = merged.get(k);
          merged.set(k, { kind: prev?.kind === 'run' || kind === 'run' ? 'run' : 'build', targets: [...new Set([...(prev?.targets ?? []), target])] });
          if (c.direct.has(id)) direct.add(k);
        }
      }
      for (const [k, v] of [...merged].sort()) {
        entry.cargo.push({ k, kind: v.kind, direct: direct.has(k), targets: v.targets.length === prog.cargo.targets.length ? null : v.targets });
        if (prog.cargo.lock === 'core') reached.core.add(k);
      }
    }
    if (prog.npm) {
      const merged = new Map();
      const direct = new Set();
      for (const imp of prog.npm) {
        const c = npmClosure(pnpm, imp);
        for (const [k, kind] of c.kinds) merged.set(k, merged.get(k) === 'prod' ? 'prod' : kind);
        c.direct.forEach((k) => direct.add(k));
      }
      for (const [k, kind] of [...merged].sort()) entry.npm.push({ k, kind: kind === 'prod' ? 'run' : 'dev', direct: direct.has(k) });
    }
    if (prog.gradle) entry.other = gradle();
    if (prog.swift) entry.other = swift();
    if (prog.vendor) entry.other = vendored();
    if (prog.ci) entry.other = ci();
    programs.push(entry);
  }
  // Core packages no shipped program reaches: test and development only.
  const rest = programs.find((p) => p.id === 'core-tests');
  for (const p of cargo.core) {
    const k = cargoKey(p.name, p.version);
    if (!packages[k] || reached.core.has(k)) continue;
    rest.cargo.push({ k, kind: 'dev', direct: false, targets: null });
  }
  const coreMembers = new Set(full.core.workspace_members);
  for (const id of full.core.workspace_members) {
    const node = full.core.resolve.nodes.find((n) => n.id === id);
    for (const d of node.deps) {
      if (coreMembers.has(d.pkg)) continue;
      const p = full.core.packages.find((x) => x.id === d.pkg);
      const k = cargoKey(p.name, p.version);
      const e = rest.cargo.find((x) => x.k === k);
      if (e) e.direct = true;
    }
  }
  rest.cargo.sort((a, b) => a.k.localeCompare(b.k));
  // npm package metadata.
  for (const [k, p] of npmPackages(pnpm)) {
    const m = npmMeta(p.name, p.version);
    packages[k] = { eco: 'npm', name: p.name, version: p.version, license: m?.license ?? '', description: clean(m?.description), repository: m?.repository ?? null, integrity: p.integrity, platform: p.os || p.cpu ? [p.os, p.cpu].filter(Boolean).join(' ') : null };
  }
  const data = {
    $comment: 'Generated by apps/site/scripts/sbom.mjs from the lockfiles; checked by --check. Do not edit.',
    lockfiles: Object.fromEntries([...Object.entries(cargo).map(([l, ps]) => [`${l}/Cargo.lock`, ps.length]), ['pnpm-lock.yaml', Object.keys(pnpm.packages ?? {}).length]]),
    programs: PROGRAMS.map((p) => ({ ...p, cargo: undefined, npm: undefined, lock: p.cargo ? `${p.cargo.lock}/Cargo.lock` : p.npm ? 'pnpm-lock.yaml' : null, targets: p.cargo?.targets ?? null, importers: p.npm ?? null, ...programs.find((x) => x.id === p.id) })),
    packages: Object.fromEntries(Object.entries(packages).sort(([a], [b]) => a.localeCompare(b))),
  };
  // One package or program entry per line: small diffs when a lockfile changes.
  const line = (v) => JSON.stringify(v);
  const text =
    '{\n' +
    `"$comment": ${line(data.$comment)},\n` +
    `"lockfiles": ${line(data.lockfiles)},\n` +
    '"programs": [\n' +
    data.programs.map((p) => {
      const { cargo: c, npm: n, other: o, ...head } = p;
      return ` {"head": ${line(head)},\n  "cargo": [\n${c.map((x) => '   ' + line(x)).join(',\n')}\n  ],\n  "npm": [\n${n.map((x) => '   ' + line(x)).join(',\n')}\n  ],\n  "other": [\n${o.map((x) => '   ' + line(x)).join(',\n')}\n  ]}`;
    }).join(',\n') +
    '\n],\n"packages": {\n' +
    Object.entries(data.packages).map(([k, v]) => ` ${line(k)}: ${line(v)}`).join(',\n') +
    '\n}\n}\n';
  writeFileSync(OUT, text);
  const used = new Set(data.programs.flatMap((p) => [...p.cargo, ...p.npm].map((x) => x.k)));
  console.log(`wrote ${relative(repo, OUT)}: ${data.programs.length} programs, ${used.size} packages in use`);
}

/* ---------- checking (no cargo) ---------- */

/** sbom.json, with each program's header folded back into it. */
export function loadSbom(file = OUT) {
  const raw = JSON.parse(readFileSync(file, 'utf8'));
  return { ...raw, programs: raw.programs.map((p) => ({ ...p.head, cargo: p.cargo, npm: p.npm, other: p.other })) };
}

export function check() {
  const data = loadSbom();
  const { cargo, pnpm } = lockfiles();
  const problems = [];
  for (const [lock, ps] of Object.entries(cargo)) {
    for (const p of ps) {
      const k = cargoKey(p.name, p.version);
      const known = data.packages[k];
      const isMember = !p.source && !known;
      if (isMember) continue; // workspace members and path crates
      if (!known) problems.push(`${lock}/Cargo.lock: ${p.name} ${p.version} is not in sbom.json`);
      else if (p.checksum && known.checksum && known.checksum !== p.checksum) problems.push(`${p.name} ${p.version}: checksum changed`);
    }
  }
  for (const [k, p] of npmPackages(pnpm)) {
    if (!data.packages[k]) problems.push(`pnpm-lock.yaml: ${p.name} ${p.version} is not in sbom.json`);
    else if (p.integrity && data.packages[k].integrity !== p.integrity) problems.push(`${p.name} ${p.version}: integrity changed`);
  }
  for (const prog of data.programs) {
    for (const e of [...prog.cargo, ...prog.npm]) if (!data.packages[e.k]) problems.push(`${prog.id}: ${e.k} has no package entry`);
  }
  const lockCounts = Object.fromEntries([...Object.entries(cargo).map(([l, ps]) => [`${l}/Cargo.lock`, ps.length]), ['pnpm-lock.yaml', Object.keys(pnpm.packages ?? {}).length]]);
  for (const [f, n] of Object.entries(lockCounts)) if (data.lockfiles[f] !== n) problems.push(`${f}: ${n} packages, sbom.json was made from ${data.lockfiles[f]}`);
  return problems;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.includes('--check')) {
    const problems = check();
    if (problems.length) {
      console.error(problems.slice(0, 40).map((p) => '✗ ' + p).join('\n'));
      console.error(`\nThe SBOM is out of date (${problems.length} problem(s)). Run: node apps/site/scripts/sbom.mjs`);
      process.exit(1);
    }
    console.log('✓ the SBOM matches every lockfile');
  } else {
    regenerate();
  }
}
