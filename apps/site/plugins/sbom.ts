import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import type { Plugin } from 'vite';

/**
 * The SBOM page's content and downloads, from apps/site/wiki/sbom/sbom.json (generated from the
 * lockfiles by scripts/sbom.mjs and checked against them by `sbom.mjs --check`):
 *   <!-- @sbom -->                the programs, every package, licenses and other ecosystems
 *   /wiki/sbom/hoi.cdx.json       CycloneDX 1.5 (JSON), one component per package, with purls
 *   /wiki/sbom/sbom.json          the generator's own data, programs and closures included
 */
interface Entry { k: string; kind: 'run' | 'build' | 'dev'; direct: boolean; targets?: string[] | null }
interface Other { eco: string; name: string; version: string; url?: string; license?: string; description?: string; path?: string; upstream?: string; sha?: string; digest?: string | null; file?: string; group?: string; artifact?: string; bom?: boolean }
interface Program { id: string; title: string; ships: boolean; wiki: string; summary: string; lock: string | null; targets: string[] | null; importers: string[] | null; cargo: Entry[]; npm: Entry[]; other: Other[] }
interface Pkg { eco: 'cargo' | 'npm'; name: string; version: string; license: string; description: string; repository: string | null; checksum?: string | null; integrity?: string | null; vendored?: string | null; local?: string | null; platform?: string | null }
interface Sbom { lockfiles: Record<string, number>; programs: Program[]; packages: Record<string, Pkg> }

const COLORS: Record<string, string> = { 'hoi-node': 'sun', 'browser-core': 'coral', 'web-app': 'coral', website: 'peach', desktop: 'sky', 'node-setup': 'sky', 'mobile-core': 'lilac', android: 'mint', ios: 'mint', 'hoi-sim': 'mint', 'core-tests': 'lilac', fuzz: 'lilac', tooling: 'peach', vendored: 'coral', ci: 'sun' };
const TARGET_NAMES: Record<string, string> = {
  'x86_64-unknown-linux-gnu': 'Linux x86-64', 'aarch64-unknown-linux-gnu': 'Linux ARM64', 'wasm32-unknown-unknown': 'WebAssembly',
  'aarch64-apple-darwin': 'macOS ARM', 'x86_64-apple-darwin': 'macOS Intel', 'x86_64-pc-windows-msvc': 'Windows x86-64',
  'aarch64-linux-android': 'Android ARM64', 'aarch64-apple-ios': 'iOS ARM64',
};

const esc = (s: string) => String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const idOf = (k: string) => 'pkg-' + k.replace(/[^a-zA-Z0-9]+/g, '-').replace(/^-|-$/g, '').toLowerCase();

function load(repo: string): Sbom {
  const raw = JSON.parse(readFileSync(repo + 'apps/site/wiki/sbom/sbom.json', 'utf8'));
  return { ...raw, programs: raw.programs.map((p: { head: object; cargo: Entry[]; npm: Entry[]; other: Other[] }) => ({ ...p.head, cargo: p.cargo, npm: p.npm, other: p.other })) };
}

/** How many distinct packages the programs use (the SBOM page's headline number). */
export function sbomCount(repoRoot: string): number {
  const d = load(repoRoot.endsWith('/') ? repoRoot : repoRoot + '/');
  return new Set(d.programs.flatMap((p) => [...p.cargo, ...p.npm].map((e) => e.k))).size;
}

function registryUrl(p: Pkg): string {
  return p.eco === 'cargo' ? `https://crates.io/crates/${p.name}/${p.version}` : `https://www.npmjs.com/package/${p.name}/v/${p.version}`;
}
function purl(p: Pkg): string {
  if (p.eco === 'cargo') return `pkg:cargo/${p.name}@${p.version}`;
  const [scope, name] = p.name.startsWith('@') ? p.name.slice(1).split('/') : [null, p.name];
  return `pkg:npm/${scope ? '%40' + scope + '/' : ''}${name}@${p.version}`;
}

export function renderSbom(repoRoot: string, repoUrl: string): string {
  const repo = repoRoot.endsWith('/') ? repoRoot : repoRoot + '/';
  const d = load(repo);
  const used = new Map<string, { prog: Program; e: Entry }[]>();
  for (const prog of d.programs) for (const e of [...prog.cargo, ...prog.npm]) {
    if (!used.has(e.k)) used.set(e.k, []);
    used.get(e.k)!.push({ prog, e });
  }
  const keys = [...used.keys()].sort((a, b) => {
    const pa = d.packages[a], pb = d.packages[b];
    return pa.name.localeCompare(pb.name) || a.localeCompare(b);
  });
  const pk = keys.map((k) => d.packages[k]);
  const cargoN = pk.filter((p) => p.eco === 'cargo' && !p.local).length;
  const npmN = pk.filter((p) => p.eco === 'npm').length;
  const licenses = new Map<string, number>();
  for (const p of pk) if (!p.local) licenses.set(p.license || 'unknown', (licenses.get(p.license || 'unknown') ?? 0) + 1);
  const shipped = new Set(keys.filter((k) => used.get(k)!.some((u) => u.prog.ships && u.e.kind === 'run')));
  const others = d.programs.reduce((n, p) => n + p.other.length, 0);

  const stats =
    `<div class="wk-stats">` +
    `<div class="wk-stat c-sun"><b>${keys.length.toLocaleString('en-US')}</b><span>packages in use</span></div>` +
    `<div class="wk-stat c-coral"><b>${cargoN}</b><span>Rust crates</span></div>` +
    `<div class="wk-stat c-sky"><b>${npmN}</b><span>npm packages</span></div>` +
    `<div class="wk-stat c-mint"><b>${shipped.size}</b><span>in shipped code</span></div>` +
    `<div class="wk-stat c-lilac"><b>${licenses.size}</b><span>license expressions</span></div>` +
    `<div class="wk-stat c-peach"><b>${d.programs.length}</b><span>programs and tool sets</span></div>` +
    `</div>`;

  const downloads =
    `<div class="wk-cards">` +
    `<a class="wk-card g-sun" href="/wiki/sbom/hoi.cdx.json" download="hoi.cdx.json"><i></i><span class="k">CycloneDX 1.5 · JSON</span><b>hoi.cdx.json</b><span>Every package as a component with its purl, license, hashes and source, for scanners and audits.</span></a>` +
    `<a class="wk-card g-coral" href="/wiki/sbom/sbom.json" download="hoi-sbom.json"><i></i><span class="k">generator data · JSON</span><b>sbom.json</b><span>The data behind this page, including which program reaches which package, per target platform.</span></a>` +
    `</div>`;

  const lockRows = Object.entries(d.lockfiles).map(([f, n]) => `<tr><td><code>${esc(f)}</code></td><td>${n}</td></tr>`).join('');

  const programCards = d.programs.map((p) => {
    const all = [...p.cargo, ...p.npm];
    const run = all.filter((e) => e.kind === 'run').length;
    const build = all.filter((e) => e.kind === 'build').length;
    const dev = all.filter((e) => e.kind === 'dev').length;
    const direct = all.filter((e) => e.direct).map((e) => d.packages[e.k]).filter((x) => !x.local).sort((a, b) => a.name.localeCompare(b.name));
    const counts = all.length
      ? `${all.length} packages · ${run} runtime${build ? ` · ${build} build-time` : ''}${dev ? ` · ${dev} development` : ''}`
      : `${p.other.length} entries`;
    const targets = p.targets ? `<p class="sb-tg">Resolved for ${p.targets.map((t) => `<code>${esc(TARGET_NAMES[t] ?? t)}</code>`).join(' ')} from <code>${esc(p.lock ?? '')}</code></p>` : p.importers ? `<p class="sb-tg">From <code>pnpm-lock.yaml</code>: ${p.importers.map((i) => `<code>${esc(i === '.' ? '(root)' : i)}</code>`).join(' ')}</p>` : '';
    return (
      `<details class="sb-prog g-${COLORS[p.id] ?? 'sun'}" id="prog-${p.id}"><summary><b>${esc(p.title)}</b>` +
      `<span class="tag ${p.ships ? 't-mint' : 't-soft'}">${p.ships ? 'ships' : 'development'}</span><span class="sb-n">${counts}</span></summary>` +
      `<p>${esc(p.summary)} <a href="${p.wiki}">More about it</a>.</p>${targets}` +
      (direct.length ? `<p class="sb-direct"><span>Declared directly:</span> ${direct.map((x) => `<a href="#${idOf((x.eco === 'cargo' ? 'cargo:' : 'npm:') + x.name + '@' + x.version)}">${esc(x.name)} <span>${esc(x.version)}</span></a>`).join(' ')}</p>` : '') +
      (p.other.length ? otherTable(p, repoUrl) : '') +
      `</details>`
    );
  }).join('');

  const progOptions = d.programs.filter((p) => p.cargo.length + p.npm.length).map((p) => `<option value="${p.id}">${esc(p.title)}</option>`).join('');
  const rows = keys.map((k) => {
    const p = d.packages[k];
    const us = used.get(k)!;
    const tags = us.map(({ prog, e }) => {
      const t = e.targets?.length ? `<small>${e.targets.map((x) => TARGET_NAMES[x] ?? x).join(', ')}</small>` : '';
      return `<span class="tag t-${e.kind === 'run' ? 'mint' : e.kind === 'build' ? 'sky' : 'soft'}">${esc(prog.title)}${e.direct ? ' ★' : ''}${t}</span>`;
    }).join(' ');
    const name = p.local
      ? `<a href="${repoUrl}/tree/main/${esc(p.local)}">${esc(p.name)}</a><span class="sb-dir">first-party</span>`
      : `<a href="${registryUrl(p)}" rel="noopener">${esc(p.name)}</a>${p.vendored ? `<span class="sb-dir patch">vendored</span>` : ''}`;
    const links = [
      p.repository ? `<a href="${esc(p.repository)}" rel="noopener">source</a>` : '',
      p.eco === 'cargo' && !p.local ? `<a href="https://docs.rs/${p.name}/${p.version}" rel="noopener">docs</a>` : '',
      p.vendored ? `<a href="${repoUrl}/tree/main/${esc(p.vendored)}">patch</a>` : '',
    ].filter(Boolean).join(' ');
    return (
      `<tr id="${idOf(k)}" data-e="${p.eco}" data-p="${us.map((u) => u.prog.id).join(' ')}" data-s="${shipped.has(k) ? 1 : 0}">` +
      `<td>${name}${p.description ? `<div class="sb-d">${esc(p.description)}</div>` : ''}</td>` +
      `<td class="sb-v">${esc(p.version)}</td><td class="sb-l">${esc(p.license || '—')}</td>` +
      `<td class="sb-u">${tags}</td><td class="sb-src">${links}</td></tr>`
    );
  }).join('');

  const licRows = [...licenses].sort((a, b) => b[1] - a[1]).map(([l, n]) => `<tr><td class="sb-l">${esc(l)}</td><td>${n}</td></tr>`).join('');

  return (
    stats + downloads +
    `<section class="wk-sec" id="programs"><h2>Programs</h2>` +
    `<p>Each program's list is its dependency closure: what its build actually compiles or bundles, resolved for the platforms it ships on. Rust closures follow normal and build edges from the program's own crate (<code>cargo metadata --filter-platform</code>), npm closures follow an importer's dependencies in <code>pnpm-lock.yaml</code>. <b>Runtime</b> packages end up in what you run; <b>build-time</b> ones (build scripts, procedural macros) only run while compiling; <b>development</b> ones only build or test. ★ marks a dependency the program declares itself.</p>` +
    `<div class="sb-progs">${programCards}</div></section>` +
    `<section class="wk-sec" id="packages"><h2>Every package</h2>` +
    `<p>One row per package and version, linked to its registry page, its source and (for crates) its documentation. The tags say which programs use it and how. Hover a tag for the details.</p>` +
    `<div class="sb-tools js-only"><input class="sb-q" type="search" placeholder="Filter by name, license or description" aria-label="Filter packages" data-sb-q>` +
    `<select class="sb-sel" aria-label="Program" data-sb-p><option value="">Every program</option>${progOptions}</select>` +
    `<div class="sb-filters" role="group" aria-label="Show"><button type="button" class="wk-btn on" data-sb-e="" aria-pressed="true">All</button><button type="button" class="wk-btn" data-sb-e="cargo" aria-pressed="false">Rust</button><button type="button" class="wk-btn" data-sb-e="npm" aria-pressed="false">npm</button><button type="button" class="wk-btn" data-sb-s="1" aria-pressed="false">Shipped only</button></div></div>` +
    `<p class="sb-count" aria-live="polite" data-sb-count>${keys.length} packages</p>` +
    `<div class="wk-table sb-all" tabindex="0" role="region" aria-label="Every package"><table><thead><tr><th>Package</th><th>Version</th><th>License</th><th>Used by</th><th>Links</th></tr></thead><tbody>${rows}</tbody></table></div></section>` +
    `<section class="wk-sec" id="licenses"><h2>Licenses</h2><p>License expressions as each package declares them (SPDX, from <code>Cargo.toml</code> or <code>package.json</code>), counted over the packages above. The cargo-deny policies decide which are allowed; see <a href="/wiki/supply-chain/">Supply chain &amp; hardening</a>.</p>` +
    `<div class="wk-table narrow" tabindex="0" role="region" aria-label="Licenses"><table><thead><tr><th>License</th><th>Packages</th></tr></thead><tbody>${licRows}</tbody></table></div></section>` +
    `<section class="wk-sec" id="sources"><h2>Where this comes from</h2>` +
    `<p>The lockfiles are the source of truth for every name, version and checksum. <code>node apps/site/scripts/sbom.mjs</code> resolves the closures with Cargo and reads each package's license, description and repository; <code>node apps/site/scripts/sbom.mjs --check</code> (part of <code>pnpm check:quick</code>) fails when a lockfile changes without the SBOM. This page and the downloads are rendered from that data when the site is built.</p>` +
    `<div class="wk-table narrow" tabindex="0" role="region" aria-label="Lockfiles"><table><thead><tr><th>Lockfile</th><th>Packages locked</th></tr></thead><tbody>${lockRows}</tbody></table></div></section>`
  );
}

function otherTable(p: Program, repoUrl: string): string {
  const rows = p.other.map((o) => {
    const name = o.url || o.upstream ? `<a href="${esc(o.url ?? o.upstream ?? '')}" rel="noopener">${esc(o.name)}</a>` : esc(o.name);
    const kind = { 'github-action': 'GitHub Action', 'cargo-tool': 'cargo install', tool: 'toolchain', docker: 'container image', maven: o.bom ? 'Maven (BOM)' : 'Maven', 'gradle-plugin': 'Gradle plugin', swift: 'Swift package', cargo: 'crate', npm: 'npm' }[o.eco] ?? o.eco;
    const pin = o.sha ? `<code>${o.sha.slice(0, 12)}</code>` : o.digest ? `<code>${esc(o.digest.slice(0, 19))}…</code>` : o.path ? `<a href="${repoUrl}/tree/main/${esc(o.path)}"><code>${esc(o.path)}</code></a>` : '';
    const note = o.description ? esc(o.description) : o.file ? `<code>${esc(o.file)}</code>` : o.license ? esc(o.license) : '';
    return `<tr><td>${name}</td><td class="sb-v">${esc(o.version)}</td><td>${kind}</td><td>${pin}</td><td class="sb-d">${note}${o.license && o.description ? ` · ${esc(o.license)}` : ''}</td></tr>`;
  }).join('');
  return `<div class="wk-table" tabindex="0" role="region" aria-label="${esc(p.title)}"><table><thead><tr><th>Name</th><th>Version</th><th>Kind</th><th>Pinned to</th><th>Notes</th></tr></thead><tbody>${rows}</tbody></table></div>`;
}

/** CycloneDX 1.5 JSON: components with purls, licenses, hashes and references. */
function cyclonedx(repo: string, version: string, repoUrl: string): string {
  const d = load(repo);
  const used = new Set(d.programs.flatMap((p) => [...p.cargo, ...p.npm].map((e) => e.k)));
  const scopeOf = (k: string) => (d.programs.some((p) => p.ships && [...p.cargo, ...p.npm].some((e) => e.k === k && e.kind === 'run')) ? 'required' : 'excluded');
  const components = [...used].sort().map((k) => {
    const p = d.packages[k];
    const hashes = [] as { alg: string; content: string }[];
    if (p.checksum) hashes.push({ alg: 'SHA-256', content: p.checksum });
    if (p.integrity?.startsWith('sha512-')) hashes.push({ alg: 'SHA-512', content: Buffer.from(p.integrity.slice(7), 'base64').toString('hex') });
    const refs = [] as { type: string; url: string }[];
    if (p.repository) refs.push({ type: 'vcs', url: p.repository });
    if (!p.local) refs.push({ type: 'distribution', url: registryUrl(p) });
    const lic = p.license && !/see license file/.test(p.license) ? [{ expression: p.license.replace(/\//g, ' OR ') }] : undefined;
    return {
      type: 'library',
      'bom-ref': purl(p),
      name: p.name,
      version: p.version,
      ...(p.description ? { description: p.description } : {}),
      scope: scopeOf(k),
      purl: purl(p),
      ...(lic ? { licenses: lic } : {}),
      ...(hashes.length ? { hashes } : {}),
      ...(refs.length ? { externalReferences: refs } : {}),
      properties: d.programs.filter((pr) => [...pr.cargo, ...pr.npm].some((e) => e.k === k)).map((pr) => ({ name: 'hoi:program', value: pr.id })),
      ...(p.vendored ? { pedigree: { notes: `Patched copy in ${p.vendored}` } } : {}),
    };
  });
  const bom = {
    bomFormat: 'CycloneDX',
    specVersion: '1.5',
    serialNumber: 'urn:uuid:' + uuidFrom(JSON.stringify(components)),
    version: 1,
    metadata: {
      tools: { components: [{ type: 'application', name: 'hoi sbom.mjs', version }] },
      component: { type: 'application', 'bom-ref': 'hoi', name: 'hoi', version, licenses: [{ license: { id: 'AGPL-3.0-only' } }], externalReferences: [{ type: 'vcs', url: repoUrl }] },
    },
    components,
  };
  return JSON.stringify(bom, null, 1) + '\n';
}

function uuidFrom(s: string): string {
  const h = createHash('sha256').update(s).digest('hex');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-5${h.slice(13, 16)}-${((parseInt(h[16], 16) & 3) | 8).toString(16)}${h.slice(17, 20)}-${h.slice(20, 32)}`;
}

export function sbomAssets(repoRoot: string, opts: { version: string; repoUrl: string }): Plugin {
  const repo = repoRoot.endsWith('/') ? repoRoot : repoRoot + '/';
  return {
    name: 'hoi-sbom',
    apply: 'build',
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'wiki/sbom/hoi.cdx.json', source: cyclonedx(repo, opts.version, opts.repoUrl) });
      this.emitFile({ type: 'asset', fileName: 'wiki/sbom/sbom.json', source: readFileSync(repo + 'apps/site/wiki/sbom/sbom.json', 'utf8') });
    },
    configureServer(server) {
      server.middlewares.use('/wiki/sbom/hoi.cdx.json', (_req, res) => {
        res.setHeader('Content-Type', 'application/json');
        res.end(cyclonedx(repo, opts.version, opts.repoUrl));
      });
      server.middlewares.use('/wiki/sbom/sbom.json', (_req, res) => {
        res.setHeader('Content-Type', 'application/json');
        res.end(readFileSync(repo + 'apps/site/wiki/sbom/sbom.json', 'utf8'));
      });
    },
  };
}
