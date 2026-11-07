// Builds the signed release list the website and the apps read (apps/site/public/releases/manifest.json
// schema): every downloadable file with its platform, size and SHA-256.
// Usage: node scripts/release-manifest.mjs <dir with release files> <tag> > manifest.json
// The files' addresses are the GitHub release's; HOI_RELEASE_BASE_URL (https://…/) puts them
// anywhere else the files are hosted, which a private repository needs for anyone to download
// them (deploy/install reads these addresses).
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

import { fileURLToPath } from 'node:url';
import { loadPublication } from './publication.mjs';
import { loadBuildRelease } from './release-config.mjs';
import { parseManifest } from './release-verify.mjs';

const SETUP = /^hoi[ ._-]node[ ._-]setup_/i;

export function classify(name) {
  const arch = /aarch64|arm64/i.test(name) ? 'aarch64' : /universal/i.test(name) ? 'universal' : 'x86_64';
  // The node setup app (apps/setup) ships beside the hoi app for the same platforms; `product`
  // keeps it off the app rows and out of the installers' app choice. Its product name is
  // "hoi Node Setup": Tauri names its bundles with spaces, a GitHub release turns them into dots,
  // and release-local.mjs and build-all.mjs copy them as hoi-node-setup_….
  const setup = SETUP.test(name);
  const product = setup ? { product: 'setup' } : {};
  if (name.endsWith('.app.tar.gz')) return { platform: 'macos', kind: 'app.tar.gz', arch, ...product };
  if (name.endsWith('.AppImage.tar.gz')) return { platform: 'linux', kind: 'AppImage.tar.gz', arch, ...product };
  if (name.endsWith('.dmg')) return { platform: 'macos', kind: 'dmg', arch, ...product };
  if (name.endsWith('.msi')) return { platform: 'windows', kind: 'msi', arch, ...product };
  if (name.endsWith('-setup.exe') || name.endsWith('.exe')) return { platform: 'windows', kind: 'exe', arch, ...product };
  if (name.endsWith('.AppImage')) return { platform: 'linux', kind: 'AppImage', arch, ...product };
  if (name.endsWith('.deb')) return { platform: 'linux', kind: 'deb', arch, ...product };
  if (name.endsWith('.rpm')) return { platform: 'linux', kind: 'rpm', arch, ...product };
  if (name.startsWith('hoi-node_') && !setup && name.endsWith('.tar.gz')) return { platform: 'node', kind: 'tar.gz', arch };
  return null;
}

export function buildManifest(dir, tag, publication = loadPublication(), release = loadBuildRelease()) {
const version = tag.replace(/^v/, '');
const repo = publication.repository || `https://github.com/${process.env.GITHUB_REPOSITORY ?? 'hoi-project/hoi'}`;
const base = process.env.HOI_RELEASE_BASE_URL || (publication.downloads ? `${publication.downloads.replace(/\/+$/, '')}/${tag}/` : '');
if (base && !/^https:\/\/[^\s?#]+\/$/.test(base)) {
  throw new Error('HOI_RELEASE_BASE_URL must be an https:// address ending in /');
}
const fileUrl = (name) => (base || `${repo}/releases/download/${tag}/`) + encodeURIComponent(name);

const files = readdirSync(dir)
  .filter((n) => statSync(join(dir, n)).isFile())
  .map((name) => {
    const c = classify(name);
    if (!c) return null;
    const bytes = readFileSync(join(dir, name));
    return {
      ...c,
      name,
      url: fileUrl(name),
      size: bytes.length,
      sha256: createHash('sha256').update(bytes).digest('hex'),
    };
  })
  .filter(Boolean)
  .sort((a, b) => (a.platform + a.kind).localeCompare(b.platform + b.kind));

const manifest = {
  schema: 1,
  latest: version,
  signature: 'manifest.json.minisig',
  release,
  releases: [{ version, date: new Date().toISOString().slice(0, 10), notes: `${repo}/releases/tag/${tag}`, files }],
};
const platforms = {};
for (const file of files) {
  if (file.product === 'setup' || !existsSync(join(dir, `${file.name}.sig`))) continue;
  const os = { macos: 'darwin', windows: 'windows', linux: 'linux' }[file.platform];
  const installer = { 'app.tar.gz': '', exe: '-nsis', msi: '-msi', AppImage: '', 'AppImage.tar.gz': '' }[file.kind];
  if (!os || installer === undefined) continue;
  const signature = readFileSync(join(dir, `${file.name}.sig`), 'utf8').trim();
  if (!signature || signature.length > 8192) throw new Error(`Invalid updater signature: ${file.name}`);
  for (const arch of file.arch === 'universal' ? ['aarch64', 'x86_64'] : [file.arch]) {
    const target = `${os}-${arch}${installer}`;
    if (platforms[target]) throw new Error(`More than one updater artifact for ${target}.`);
    platforms[target] = { url: file.url, signature };
  }
}
manifest.updater = { version, notes: release.notice.message, platforms };
parseManifest(new TextEncoder().encode(JSON.stringify(manifest)));
return manifest;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const [dir, tag] = process.argv.slice(2);
  if (!dir || !tag) throw new Error('usage: release-manifest.mjs <dir> <tag>');
  process.stdout.write(JSON.stringify(buildManifest(dir, tag), null, 2) + '\n');
}
