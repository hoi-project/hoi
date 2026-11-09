// Match the registry chosen for this build with one exact CSP origin. No wildcard origins.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { run } from '@tauri-apps/cli';
import { relayUrls } from '../../../scripts/relay-config.mjs';
import { linuxBundles, windowsBundles } from '../../../scripts/tauri-bundles.mjs';
import { loadPublication } from '../../../scripts/publication.mjs';
import { clearEmptyAppleSigningEnv } from '../../../scripts/tauri-signing.mjs';

export function registryOverride(base, configured) {
  if (!configured) return null;
  const url = new URL(configured);
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname))) {
    throw new Error('VITE_HOI_REGISTRY_URL must use HTTPS (except loopback development).');
  }
  if (url.username || url.password || url.search || url.hash) throw new Error('Registry URLs cannot include credentials, queries or fragments.');
  const csp = { ...base.app.security.csp };
  csp['connect-src'] = [...new Set([...csp['connect-src'].split(' '), url.origin])].join(' ');
  return { app: { security: { csp } } };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const publication = loadPublication();
  const base = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  let override = registryOverride(base, process.env.VITE_HOI_REGISTRY_URL);
  for (const relay of relayUrls(process.env.VITE_HOI_RELAY_URLS)) {
    if (relay === '/relay') throw new Error('Desktop relay URLs must be absolute HTTPS addresses.');
    override = registryOverride(override ? { app: { security: { csp: { ...base.app.security.csp, ...override.app.security.csp } } } } : base, relay);
  }
  if (process.env.HOI_PUBLICATION === '1' && args[0] === 'build') {
    if (!process.env.TAURI_SIGNING_PRIVATE_KEY) throw new Error('Publication builds require the updater signing key (TAURI_SIGNING_PRIVATE_KEY).');
    override = { ...override, bundle: { createUpdaterArtifacts: true }, plugins: { updater: { pubkey: Buffer.from(`untrusted comment: hoi updater public key\n${publication.updaterKey}\n`).toString('base64') } } };
  }
  if (override && ['dev', 'build'].includes(args[0])) args.push('--config', JSON.stringify(override));
  // A preview version is built for Windows without the .msi, which takes numeric versions only.
  const windows = windowsBundles(args, base.version);
  const linux = linuxBundles(windows.args, base.version);
  for (const note of [windows.note, linux.note]) if (note) console.error(note);
  clearEmptyAppleSigningEnv();
  await run(linux.args, 'pnpm tauri');
}
