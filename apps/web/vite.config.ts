import { fileURLToPath } from 'node:url';
import { readFileSync } from 'node:fs';
import { defineConfig, loadEnv, type Plugin } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import hardening, { headers } from '@hoi/vite-plugin-hardening';
import serviceWorker from './vite/sw-plugin.js';
import pkg from './package.json' with { type: 'json' };
import { loadBuildRelease, loadRelease } from '../../scripts/release-config.mjs';
import { relayUrls } from '../../scripts/relay-config.mjs';
import { loadPublication, configureInstaller } from '../../scripts/publication.mjs';

// `--mode desktop` (or HOI_TARGET=desktop) builds the UI for the Tauri shell: Tauri sets its own
// CSP and there is no service worker. `--mode setup` builds the node setup app's window
// (apps/setup) from the `setup/` folder: one page, no core, no worker, no service worker.
const isDesktop = (mode: string) => mode === 'desktop' || process.env.HOI_TARGET === 'desktop';
const isSetup = (mode: string) => mode === 'setup';

// One versioned manifest controls all surfaces. Changing a notice never deletes local data.
const RELEASE = loadBuildRelease();
const PREVIEW = RELEASE.channel === 'preview';
const SANDBOX = process.env.HOI_SANDBOX === '1';
const PUBLICATION = loadPublication();

function installerTrust(): Plugin {
  return { name: 'hoi-installer-trust', enforce: 'pre', transform(code, id) {
    if (/\/deploy\/install\/install\.(sh|ps1)\?raw$/.test(id)) return { code: `export default ${JSON.stringify(configureInstaller(readFileSync(id.slice(0, -4), 'utf8'), PUBLICATION))}`, map: null };
  } };
}

const CSP: Record<string, string[]> = {
  'default-src': ["'self'"],
  'script-src': ["'self'", "'wasm-unsafe-eval'"],
  'worker-src': ["'self'"],
  'style-src': ["'self'"],
  'img-src': ["'self'", 'data:', 'blob:'],
  // media plays only from bytes the core checked and handed over (blob: URLs), never from the network
  'media-src': ["'self'", 'blob:'],
  'font-src': ["'self'"],
  // https: for the trackers a community names, asked only when no route answers; the
  // app itself names no service
  'connect-src': ["'self'", 'ws:', 'wss:', 'https:'],
  'object-src': ["'none'"],
  'base-uri': ["'none'"],
  'form-action': ["'none'"],
};

/** Emits `_headers` (for hosts that read it, such as Netlify) with the same policy plus headers a meta tag cannot set. */
function headersFile(): Plugin {
  return {
    name: 'hoi-headers-file',
    apply: 'build',
    generateBundle() {
      const extra = PREVIEW ? { 'X-Robots-Tag': 'noindex, nofollow' } : {};
      this.emitFile({ type: 'asset', fileName: '_headers', source: headers(CSP, extra) + '\n/release.json\n  Cache-Control: no-store\n' });
    },
  };
}

/** A preview app stays out of search results on any host, including those that ignore `_headers`; the website at / is indexed. */
function previewNoindex(): Plugin {
  return {
    name: 'hoi-preview-noindex',
    apply: 'build',
    transformIndexHtml: () => (PREVIEW ? [{ tag: 'meta', attrs: { name: 'robots', content: 'noindex, nofollow' }, injectTo: 'head' }] : []),
  };
}

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

export default defineConfig(({ mode }) => {
  const setup = isSetup(mode);
  const desktop = isDesktop(mode) || setup;
  const relays = relayUrls(loadEnv(mode, process.cwd(), 'VITE_HOI_RELAY_URLS').VITE_HOI_RELAY_URLS,
    []);
  for (const relay of relays) {
    if (relay === '/relay') continue;
    const origin = new URL(relay).origin;
    if (!CSP['connect-src'].includes(origin)) CSP['connect-src'].push(origin);
  }
  // Static builds have no service backend. Connect directly to an explicitly chosen registry.
  const registry = loadEnv(mode, process.cwd(), 'VITE_HOI_REGISTRY_URL').VITE_HOI_REGISTRY_URL
    ?? '';
  if (registry && registry.replace(/\/+$/, '') !== '/registry') {
    const url = new URL(registry);
    if (url.protocol !== 'https:' && !(url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname))) {
      throw new Error('VITE_HOI_REGISTRY_URL must use HTTPS (except loopback development).');
    }
    if (url.username || url.password || url.hash || url.search) throw new Error('Registry URLs cannot include credentials, queries or fragments.');
    if (!CSP['connect-src'].includes(url.origin)) CSP['connect-src'].push(url.origin);
  }
  return {
    // Relative base: the same build works under /app/ on the static site and inside Tauri.
    base: './',
    plugins: [
      installerTrust(),
      // The setup window's root is `setup/`; the Svelte config stays beside this file.
      svelte(setup ? { configFile: fileURLToPath(new URL('./svelte.config.js', import.meta.url)) } : {}),
      ...(setup ? [] : [releaseMetadata()]),
      hardening({ csp: desktop ? false : CSP }),
      ...(desktop ? [] : [headersFile(), previewNoindex(), ...(SANDBOX ? [] : [serviceWorker()])]),
    ],
    define: {
      __HOI_RELAY_URLS__: JSON.stringify(relays),
      'import.meta.env.VITE_HOI_REGISTRY_URL': JSON.stringify(registry),
      __HOI_DESKTOP__: JSON.stringify(desktop),
      __HOI_SETUP__: JSON.stringify(setup),
      __HOI_SANDBOX__: JSON.stringify(SANDBOX && !desktop),
      __HOI_VERSION__: JSON.stringify(pkg.version),
      __HOI_PUBLICATION__: JSON.stringify(PUBLICATION),
      // Who publishes this build and how to reach them, for the privacy notice; never in the source.
      __HOI_PUBLISHER__: JSON.stringify(process.env.HOI_PUBLISHER ?? ''),
      __HOI_CONTACT__: JSON.stringify(process.env.HOI_CONTACT ?? PUBLICATION.contact),
      __HOI_PREVIEW__: JSON.stringify(PREVIEW),
      __HOI_RELEASE__: JSON.stringify(RELEASE),
      __HOI_PROTOCOL__: JSON.stringify(RELEASE.protocolVersion),
    },
    worker: { format: 'es' },
    build: {
      target: 'es2022',
      sourcemap: false,
      ...(setup ? { outDir: fileURLToPath(new URL('./dist-setup', import.meta.url)), emptyOutDir: true } : {}),
    },
    ...(setup ? {
      root: fileURLToPath(new URL('./setup', import.meta.url)),
      publicDir: false,
      // The window's files live beside the app's; the installer it bundles is in deploy/.
      server: { fs: { allow: [fileURLToPath(new URL('../..', import.meta.url))] } },
    } : {}),
  };
});
