#!/usr/bin/env node
// Prepare release metadata without building, committing, tagging or accessing the network.
import { appendFileSync, existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadRelease, validateRelease } from './release-config.mjs';
import { releaseTier } from './release-tier.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
export const LOCKFILES = ['core/Cargo.lock', 'apps/desktop/src-tauri/Cargo.lock', 'apps/setup/src-tauri/Cargo.lock'];

/** Every first-party manifest that carries the product version. */
export function versionFiles(base = root) {
  const files = [
    { path: 'core/Cargo.toml', kind: 'cargo', section: 'workspace.package' },
    { path: 'apps/desktop/src-tauri/Cargo.toml', kind: 'cargo', section: 'package' },
    { path: 'apps/desktop/src-tauri/tauri.conf.json', kind: 'json' },
    { path: 'apps/setup/src-tauri/Cargo.toml', kind: 'cargo', section: 'package' },
    { path: 'apps/setup/src-tauri/tauri.conf.json', kind: 'json' },
  ];
  if ('version' in JSON.parse(readFileSync(join(base, 'package.json'), 'utf8'))) {
    files.push({ path: 'package.json', kind: 'json' });
  }
  for (const dir of ['apps', 'packages']) {
    for (const name of readdirSync(join(base, dir)).sort()) {
      const path = `${dir}/${name}/package.json`;
      if (existsSync(join(base, path)) && 'version' in JSON.parse(readFileSync(join(base, path), 'utf8'))) {
        files.push({ path, kind: 'json' });
      }
    }
  }
  return files;
}

function cargoVersion(text, section) {
  const start = text.indexOf(`[${section}]`);
  if (start < 0) return null;
  const rest = text.slice(start);
  const end = rest.slice(1).search(/^\[/m);
  const body = end < 0 ? rest : rest.slice(0, end + 1);
  const match = body.match(/^version = "([^"]*)"$/m);
  return match ? { version: match[1], index: start + match.index, line: match[0] } : null;
}

export function versionFromText(file, text) {
  return file.kind === 'cargo' ? (cargoVersion(text, file.section)?.version ?? null) : (JSON.parse(text).version ?? null);
}

export function readVersion(file, base = root) {
  return versionFromText(file, readFileSync(join(base, file.path), 'utf8'));
}

function versionedText(file, text, version) {
  if (file.kind === 'cargo') {
    const at = cargoVersion(text, file.section);
    if (!at) throw new Error(`${file.path}: no version in [${file.section}]`);
    return text.slice(0, at.index) + `version = "${version}"` + text.slice(at.index + at.line.length);
  }
  const next = text.replace(/^(\s{2}"version":\s*")[^"]*(")/m, `$1${version}$2`);
  if (JSON.parse(next).version !== version) throw new Error(`${file.path}: cannot locate the top-level version`);
  return next;
}

export function writeVersion(file, version, base = root) {
  const path = join(base, file.path);
  writeFileSync(path, versionedText(file, readFileSync(path, 'utf8'), version));
}

/** Update only local hoi crates; external and vendored dependency versions stay unchanged. */
export function lockWithVersion(text, version) {
  return text.replace(/^name = "(hoi-[a-z-]+)"(\r?\n)version = "[^"]*"\2(?!source = )/gm,
    (_, name, newline) => `name = "${name}"${newline}version = "${version}"${newline}`);
}

export function mismatches(version, base = root) {
  const wrong = versionFiles(base)
    .map(file => [file.path, readVersion(file, base)])
    .filter(([, found]) => found !== version)
    .map(([path, found]) => `${path} (${found ?? 'none'})`);
  for (const path of LOCKFILES) {
    const text = readFileSync(join(base, path), 'utf8');
    if (lockWithVersion(text, version) !== text) wrong.push(`${path} (local crate versions differ)`);
  }
  return wrong;
}

function normalizeVersion(value) {
  releaseTier(value);
  return value.replace(/^v/, '');
}

/** Keep the release form's static tag choices in step with normal version preparation. */
export function releaseTagChoices(text, version) {
  const tag = `v${normalizeVersion(version)}`;
  const dispatch = text.match(/^  workflow_dispatch:\r?\n((?:[ \t]*\r?\n| {4,}[^\r\n]*(?:\r?\n|$))*)/m);
  const input = dispatch?.[0].match(/^      tag:\r?\n((?: {8,}[^\r\n]*(?:\r?\n|$))*)/m);
  const options = input?.[0].match(/^        options:\r?\n((?: {10}-[^\r\n]*(?:\r?\n|$))*)/m);
  if (!input || !/^        type: choice\r?$/m.test(input[0]) || !options) {
    throw new Error('The Release workflow must define a tag choice input before preparing a version.');
  }
  const existing = options[1].trim().split(/\r?\n/).filter(Boolean).map(line => {
    const value = line.trim().replace(/^-\s*/, '').replace(/^(["'])(.*)\1$/, '$2');
    if (value !== 'Current version' && value !== `v${normalizeVersion(value)}`) {
      throw new Error('The Release tag choices must contain version tags or Current version.');
    }
    return value;
  });
  const tags = [...new Set([tag, ...existing.filter(value => value !== 'Current version')])].slice(0, 20);
  const newline = text.includes('\r\n') ? '\r\n' : '\n';
  const choices = ['"Current version"', ...tags].map(value => `          - ${value}${newline}`).join('');
  const start = dispatch.index + input.index + options.index;
  return text.slice(0, start) + `        options:${newline}${choices}` + text.slice(start + options[0].length);
}

export function releaseDetails(version, release) {
  version = normalizeVersion(version);
  validateRelease(release);
  if (version.includes('-') && release.channel !== 'preview') {
    throw new Error('A prerelease version requires channel preview.');
  }
  return { version, tag: `v${version}`, tier: releaseTier(version), channel: release.channel,
    prerelease: release.channel === 'preview' };
}

export function checkVersion(version, base = root) {
  version = normalizeVersion(version ?? readVersion(versionFiles(base)[0], base));
  const details = releaseDetails(version, loadRelease(join(base, 'release.json')));
  const wrong = mismatches(version, base);
  if (wrong.length) throw new Error(`Not version ${version}: ${wrong.join(', ')}`);
  return details;
}

const notices = {
  preview: { title: 'Preview — unstable', message: 'hoi is in development. Protocol changes may cause data loss. Do not rely on this preview to preserve data.' },
  stable: { title: 'Stable release', message: 'This build is on the stable release channel.' },
};

/** Read and validate all files before writing any of them. Compatibility changes are explicit. */
export function planVersion(value, options = {}, base = root) {
  const version = normalizeVersion(value);
  const manifest = readFileSync(join(base, 'release.json'), 'utf8');
  const original = JSON.parse(manifest);
  validateRelease(original);
  const channel = options.channel ?? (version.startsWith('0.') || version.includes('-') ? 'preview' : 'stable');
  const release = { ...original, channel, notice: { ...(channel === original.channel ? original.notice : notices[channel]) } };
  if (options.protocol !== undefined) {
    release.protocolVersion = options.protocol;
    if (options.protocol !== original.protocolVersion) release.supportedProtocols = [options.protocol];
  }
  if (options.supportedProtocols !== undefined) release.supportedProtocols = options.supportedProtocols;
  if (options.index !== undefined) release.indexVersion = options.index;
  if (options.noticeTitle !== undefined) release.notice.title = options.noticeTitle;
  if (options.noticeMessage !== undefined) release.notice.message = options.noticeMessage;
  const metadataChanged = JSON.stringify(original) !== JSON.stringify(release);
  if (metadataChanged) release.revision++;
  const details = releaseDetails(version, release);
  const changes = [];
  const add = (path, before, after) => { if (before !== after) changes.push({ path, before, after }); };
  for (const file of versionFiles(base)) {
    const before = readFileSync(join(base, file.path), 'utf8');
    add(file.path, before, versionedText(file, before, version));
  }
  for (const path of LOCKFILES) {
    const before = readFileSync(join(base, path), 'utf8');
    add(path, before, lockWithVersion(before, version));
  }
  const workflow = '.github/workflows/release.yml';
  // Minimal source exports and version-test fixtures need not contain GitHub workflow files.
  if (existsSync(join(base, workflow))) {
    const before = readFileSync(join(base, workflow), 'utf8');
    add(workflow, before, releaseTagChoices(before, version));
  }
  if (metadataChanged) {
    const newline = manifest.includes('\r\n') ? '\r\n' : '\n';
    add('release.json', manifest, (JSON.stringify(release, null, 2) + '\n').replaceAll('\n', newline));
  }
  return { ...details, release, changes };
}

export function applyVersion(plan, base = root) {
  for (const change of plan.changes) writeFileSync(join(base, change.path), change.after);
}

const help = `Usage: pnpm release:version [VERSION] [options]
  VERSION                     SemVer, e.g. 1.0.0-preview.1 (optional v prefix)
  --dry-run                   Show planned edits without writing
  --check                     Check manifests, lockfiles and release channel
  --channel preview|stable    Default: preview for 0.x/prereleases, stable otherwise
  --protocol VERSION          Set protocol; a changed protocol defaults to supporting only itself
  --supported-protocols LIST   Explicit comma-separated supported protocols
  --index NUMBER              Set registry index version
  --notice-title TEXT         Set release notice title
  --notice-message TEXT       Set release notice message
  --github-output             With --check, append release details to GITHUB_OUTPUT
No VERSION or options: check and show the current release.
Only files are edited. No commit, tag, push, build or deployment is performed.`;

function main(args) {
  if (args.length === 1 && ['--help', '-h'].includes(args[0])) { console.log(help); return; }
  const options = {};
  let version, check = false, dryRun = false, githubOutput = false;
  const valueOptions = { '--channel': 'channel', '--protocol': 'protocol', '--supported-protocols': 'supportedProtocols',
    '--index': 'index', '--notice-title': 'noticeTitle', '--notice-message': 'noticeMessage' };
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--check') check = true;
    else if (arg === '--dry-run') dryRun = true;
    else if (arg === '--github-output') githubOutput = true;
    else if (Object.hasOwn(valueOptions, arg)) {
      const value = args[++i];
      if (value === undefined || value.startsWith('--')) throw new Error(`${arg} needs a value.`);
      if (arg === '--index' && !/^[1-9]\d*$/.test(value)) throw new Error('--index needs a positive integer.');
      options[valueOptions[arg]] = arg === '--index' ? Number(value) : arg === '--supported-protocols' ? value.split(',').map(part => part.trim()) : value;
    } else if (!arg.startsWith('-') && version === undefined) version = arg;
    else throw new Error(`Unexpected argument: ${arg}`);
  }
  if (!args.length) check = true;
  if (check && (dryRun || Object.keys(options).length)) throw new Error('--check cannot be combined with editing options.');
  if (githubOutput && (!check || !process.env.GITHUB_OUTPUT)) throw new Error('--github-output requires --check and GITHUB_OUTPUT.');
  if (check) {
    const details = checkVersion(version);
    console.log(`${details.tag}: ${details.channel}, ${details.tier} checks. All manifest and lockfile versions match.`);
    if (githubOutput) appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(details).map(([key, value]) => `${key}=${value}\n`).join(''));
    return;
  }
  if (!version) throw new Error('Provide a version to edit. Use --help for options.');
  const plan = planVersion(version, options);
  if (!dryRun) applyVersion(plan);
  console.log(`${dryRun ? 'Would prepare' : 'Prepared'} ${plan.tag}: ${plan.channel}, ${plan.tier} checks, metadata revision ${plan.release.revision}.`);
  for (const change of plan.changes) console.log(`  ${change.path}`);
  if (!plan.changes.length) console.log('Already up to date.');
  if (!dryRun) console.log(`Review and commit the changes, then push to main.\nWith HOI_RELEASES_ENABLED=true in hoi-project/hoi, GitHub creates ${plan.tag} and starts the draft release automatically.\nFor a manual run, select the pushed branch in Actions → Release version; no tag is needed.\nSee docs/RELEASES.md for signing and publishing.`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try { main(process.argv.slice(2)); }
  catch (error) { console.error(`release-version: ${error.message}`); process.exitCode = 1; }
}
