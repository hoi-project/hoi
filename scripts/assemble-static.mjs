// Assembles static files only: information at / and the browser client at /app/.
// Registry discovery and peer connections run in the client; no services are bundled.
// Both pages carry their own CSP <meta>; `_headers` repeats them as headers for hosts that read
// it (Netlify and similar; GitHub Pages ignores it), scoped so the site's stricter policy never applies to /app/.
// Hosting the app on its own origin (a separate subdomain) is better still: its keys then share
// nothing with the site. Build each app's dist/ separately for that.
import { cpSync, existsSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadBuildRelease, validateRelease } from './release-config.mjs';
import { configureInstaller } from './publication.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const site = resolve(root, 'apps/site/dist');
const web = resolve(root, 'apps/web/dist');
const out = resolve(root, 'dist');
const release = loadBuildRelease();

for (const d of [site, web]) {
  if (!existsSync(d)) {
    console.error(`missing ${d}: run the builds first (pnpm build)`);
    process.exit(1);
  }
  const built = validateRelease(JSON.parse(readFileSync(resolve(d, 'release.json'), 'utf8')));
  if (JSON.stringify(built) !== JSON.stringify(release)) throw new Error(`Release metadata in ${d} differs from release.json; rebuild both site and app before assembling.`);
}
rmSync(out, { recursive: true, force: true });
cpSync(site, out, { recursive: true });
cpSync(web, resolve(out, 'app'), { recursive: true });
// The installers people fetch with curl or PowerShell (hoi.sh/install.sh, hoi.sh/install.ps1):
// static files like the rest; they download releases from the manifest and check them.
for (const name of ['install.sh', 'install.ps1']) writeFileSync(resolve(out, name), configureInstaller(readFileSync(resolve(root, 'deploy/install', name), 'utf8')));

/** Parses `_headers` into [path, lines][] blocks (comments dropped). */
const blocks = (file) => {
  if (!existsSync(file)) return [];
  const out = [];
  for (const line of readFileSync(file, 'utf8').split('\n')) {
    if (line.startsWith('#') || !line.trim()) continue;
    if (line.startsWith('  ')) out.at(-1)?.[1].push(line);
    else out.push([line.trim(), []]);
  }
  return out;
};
const siteBlocks = blocks(resolve(site, '_headers'));
const appBlocks = blocks(resolve(web, '_headers'));
rmSync(resolve(out, 'app/_headers'), { force: true });

// The site's catch-all rule is spelled out per site path so it never matches /app/.
const sitePaths = ['/', '/404.html', '/robots.txt', '/sitemap.xml', '/favicon.svg', '/og.png', '/release.json', '/preview.json', '/install.sh', '/install.ps1', '/.well-known/*'];
for (const name of readdirSync(site)) {
  if (statSync(resolve(site, name)).isDirectory() && !name.startsWith('.')) sitePaths.push(`/${name}/*`);
}
const siteAll = siteBlocks.find(([p]) => p === '/*')?.[1] ?? [];
let headers = '';
for (const p of sitePaths) {
  const extra = siteBlocks.find(([q]) => q === p)?.[1] ?? [];
  headers += `${p}\n${[...siteAll, ...extra].join('\n')}\n`;
}
for (const [p, lines] of appBlocks) headers += `${p === '/*' ? '/app/*' : '/app' + p}\n${lines.join('\n')}\n`;
writeFileSync(resolve(out, '_headers'), headers);

// The shared manifest is the authority; no environment flag can silently create a stable
// surface while the app remains a preview. Metadata never instructs a device to wipe itself.
writeFileSync(resolve(out, 'release.json'), JSON.stringify(release, null, 2) + '\n');
writeFileSync(resolve(out, 'preview.json'), JSON.stringify({
  preview: release.channel === 'preview', protocol: release.protocolVersion,
  indexVersion: release.indexVersion, revision: release.revision, note: release.notice.message,
}, null, 2) + '\n');
// robots.txt and sitemap.xml stay as the site built them: the website is indexed, preview or not,
// and a preview app keeps itself out of search results with its own robots <meta>.
const note = release.channel === 'preview' ? ' (PREVIEW: app noindex, release.json + preview.json written; local data preserved)' : '';
console.log(`assembled ${out}: site at /, app at /app/ (static files only)${note}`);
