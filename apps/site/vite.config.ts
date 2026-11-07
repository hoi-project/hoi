import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Plugin } from 'vite';
import hardening from '@hoi/vite-plugin-hardening';
import partials from './plugins/partials.ts';
import headersFile from './plugins/headers-file.ts';
import wiki, { loadWiki } from './plugins/wiki.ts';
import { renderSbom, sbomAssets, sbomCount } from './plugins/sbom.ts';
import { loadBuildRelease, loadRelease } from '../../scripts/release-config.mjs';
import { loadPublication } from '../../scripts/publication.mjs';

// Build-time configuration (TECHNICAL.md §11):
//   HOI_APP_URL        where the web app lives ("Browser" button, "Open in your browser"), default /app/
//   HOI_REPO_URL       the source repository, default https://github.com/hoi-project/hoi
//   HOI_SITE_URL       where this site is served, for the install commands, canonical links, link
//                      previews, robots.txt and the sitemap, default https://hoi.sh
//   HOI_SITE_PUBLISHER, HOI_SITE_CONTACT, HOI_SITE_HOSTER, HOI_SITE_HOSTER_PRIVACY_URL
//                      the privacy policy's responsible party, contact and hoster (defaults: the
//                      project, the repository, GitHub Pages)
// Release channel, notices and protocol/index versions come from the root release.json.
// The site is indexed whatever the channel: every page carries the release notice, and the app keeps
// itself out of search results while it is a preview (apps/web/vite.config.ts).
// @types/node isn't a dependency here, so read the environment without naming `process`.
const env = (globalThis as { process?: { env?: Record<string, string | undefined> } }).process?.env ?? {};
const PUBLICATION = loadPublication();
const APP_URL = env.HOI_APP_URL || '/app/';
const REPO_URL = (PUBLICATION.repository || env.HOI_REPO_URL || 'https://github.com/hoi-project/hoi').replace(/\/+$/, '');
const SITE_URL = (PUBLICATION.site || env.HOI_SITE_URL || 'https://hoi.sh').replace(/\/+$/, '');
// The privacy policy: who publishes the site and how to reach them, and who hosts it.
// Never in the source: public copy carries no addresses (scripts/public-copy.test.mjs).
const PUBLISHER = env.HOI_SITE_PUBLISHER || env.HOI_PUBLISHER || 'the hoi project';
const CONTACT = PUBLICATION.contact || env.HOI_SITE_CONTACT || env.HOI_CONTACT || 'through the source repository (Source, in the footer)';
const HOSTER = env.HOI_SITE_HOSTER || 'GitHub Pages (GitHub, Inc.)';
const HOSTER_PRIVACY_URL = env.HOI_SITE_HOSTER_PRIVACY_URL || 'https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement';
const PRIVACY_DATE = '2026-11-06';
const RELEASE = loadBuildRelease();
const SITE_VERSION = (JSON.parse(readFileSync(fileURLToPath(new URL('./package.json', import.meta.url)), 'utf8')) as { version: string }).version;
const root = fileURLToPath(new URL('.', import.meta.url));
const PREVIEW = RELEASE.channel === 'preview';

// Every page: its build input name, its HTML file, and its address. The wiki's pages come from wiki/nav.json.
const pages = [
  ...['index', '404', 'download', 'verify', 'relay', 'communities', 'protocol', 'privacy'].map((p) => ({
    name: p,
    file: p === 'index' || p === '404' ? `${p}.html` : `${p}/index.html`,
    url: p === 'index' ? '/' : `/${p}/`,
  })),
  ...loadWiki(root).map((p) => ({ name: p.slug ? `wiki-${p.slug}` : 'wiki', file: p.file, url: p.href })),
];

function releaseMetadata(): Plugin {
  return {
    name: 'hoi-release-metadata',
    configureServer(server) {
      server.middlewares.use('/release.json', (_req, res) => {
        res.setHeader('Content-Type', 'application/json');
        res.setHeader('Cache-Control', 'no-store');
        res.end(JSON.stringify(loadRelease()));
      });
    },
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'release.json', source: JSON.stringify(RELEASE, null, 2) + '\n' });
    },
  };
}

/** robots.txt and sitemap.xml, naming every page but the 404 by its address under HOI_SITE_URL. */
function searchMetadata(): Plugin {
  return {
    name: 'hoi-search-metadata',
    apply: 'build',
    generateBundle() {
      const urls = pages.filter((p) => p.name !== '404').map((p) => SITE_URL + p.url);
      this.emitFile({
        type: 'asset',
        fileName: 'robots.txt',
        source: `# hoi.sh is a static site with nothing to hide and nothing to track.\nUser-agent: *\nAllow: /\n\nSitemap: ${SITE_URL}/sitemap.xml\n`,
      });
      this.emitFile({
        type: 'asset',
        fileName: 'sitemap.xml',
        source:
          '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n' +
          urls.map((u) => `  <url><loc>${u}</loc></url>\n`).join('') +
          '</urlset>\n',
      });
    },
  };
}

export default defineConfig({
  root,
  appType: 'mpa',
  base: '/',
  define: { __HOI_RELEASE__: JSON.stringify(RELEASE), __HOI_PUBLICATION__: JSON.stringify(PUBLICATION) },
  build: {
    target: 'es2022',
    // Never inline fonts or images as data: URIs; the CSP only allows fonts from 'self'.
    assetsInlineLimit: 0,
    // No <link rel=modulepreload>: with scripts disabled browsers cancel those fetches, and every
    // page is just its own small entry plus the shared chunk below.
    modulePreload: false,
    rolldownOptions: {
      input: Object.fromEntries(pages.map((p) => [p.name, root + p.file])),
      output: {
        // Everything shared (Anime.js, the common module, platform detection) in one cached file.
        codeSplitting: {
          groups: [{ name: 'common', test: /[\\/]node_modules[\\/]animejs[\\/]|[\\/]src[\\/](common|platform)\.ts$/ }],
        },
      },
    },
  },
  plugins: [
    partials({
      dir: root + 'partials',
      vars: {
        APP_URL, REPO_URL, SITE_URL, PREVIEW: 'on',
        RELEASE_KEY: PUBLICATION.manifestKey || 'Not configured — no signed downloads or updates are offered.',
        RELEASE_MANIFEST: PUBLICATION.downloads ? `${PUBLICATION.downloads.replace(/\/+$/, '')}/manifest.json` : '/releases/manifest.json',
        PUBLISHER, CONTACT, HOSTER, HOSTER_PRIVACY_URL, PRIVACY_DATE,
        // Canonical address and og:url; a page that shouldn't be found passes robots="noindex" to head.html.
        robots: 'index',
        RELEASE_TITLE: RELEASE.notice.title, RELEASE_MESSAGE: RELEASE.notice.message,
        RELEASE_BUILD: `${RELEASE.channel} · protocol ${RELEASE.protocolVersion} · index ${RELEASE.indexVersion}`,
        RELEASE_WARNING: PREVIEW ? 'Unstable preview; data may be lost.' : '',
        // The wiki's home page counts its own pages and the SBOM's packages.
        WIKI_PAGES: String(loadWiki(root).length),
        SBOM_PACKAGES: sbomCount(root + '../../').toLocaleString('en-US'),
      },
    }),
    wiki({ root, sbom: () => renderSbom(root + '../../', REPO_URL) }),
    sbomAssets(root + '../../', { version: SITE_VERSION, repoUrl: REPO_URL }),
    releaseMetadata(),
    searchMetadata(),
    hardening({ csp: {
      'default-src': ["'self'"], 'script-src': ["'self'"], 'style-src': ["'self'"],
      'img-src': ["'self'", 'data:'], 'font-src': ["'self'"],
      'connect-src': ["'self'", ...(PUBLICATION.downloads ? [new URL(PUBLICATION.downloads).origin] : [])],
      'object-src': ["'none'"], 'base-uri': ["'none'"], 'form-action': ["'none'"],
    } }),
    headersFile(),
  ],
});
