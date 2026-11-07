// One build-time trust root. Never replace these keys with values fetched from a release host.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const FIELDS = ['repository', 'site', 'downloads', 'contact', 'manifestKey', 'updaterKey'];
// What each publication level needs. `site` deploys the website and browser app (a preview
// included): they name the source and their own address, and offer no signed downloads or
// updates until `downloads` and `manifestKey` are configured. `release` signs and uploads.
export const PUBLICATION_LEVELS = { site: ['repository', 'site'], release: FIELDS };

/** HOI_PUBLICATION: unset for development, `site` for a website deployment, `1` or `release` for a release. */
export function publicationLevel(value = process.env.HOI_PUBLICATION) {
  if (value === undefined || value === '' || value === '0') return undefined;
  if (value === '1' || value === 'release') return 'release';
  if (value === 'site') return 'site';
  throw new Error(`HOI_PUBLICATION must be unset, "site" or "1" (release); got ${JSON.stringify(value)}.`);
}

/** `required`: false (development), true or 'release' (every field), or 'site' (repository and site). */
export function validatePublication(value, required = false) {
  if (!value || value.schema !== 1) throw new Error('Invalid publication configuration.');
  const level = required === true ? 'release' : required || undefined;
  if (level && !PUBLICATION_LEVELS[level]) throw new Error(`Unknown publication level ${level}.`);
  const needed = level ? PUBLICATION_LEVELS[level] : [];
  const missing = [];
  for (const name of FIELDS) {
    if (typeof value[name] !== 'string' || value[name].length > 2048) throw new Error(`Invalid publication ${name}.`);
    if (needed.includes(name) && (!value[name].trim() || /OWNER|placeholder|example\.(com|org|net)/i.test(value[name]))) missing.push(name);
  }
  if (missing.length) throw new Error(`Configure publication.json: required before ${level === 'site' ? 'deploying the website' : 'publication'}: ${missing.join(', ')}. See docs/RELEASES.md; only public signing keys belong in this file.`);
  // Downloads are offered only with the key that verifies them; one without the other is a mistake.
  if (level === 'site' && !value.downloads.trim() !== !value.manifestKey.trim()) {
    throw new Error('Configure publication.json: downloads and manifestKey are set together or not at all.');
  }
  for (const name of ['repository', 'site', 'downloads']) {
    if (!value[name]) continue;
    const u = new URL(value[name]);
    if (u.protocol !== 'https:' || u.username || u.password || u.search || u.hash) throw new Error(`Publication ${name} needs an HTTPS URL without credentials, query or fragment.`);
  }
  for (const name of ['manifestKey', 'updaterKey']) {
    if (value[name] && (!/^RW[A-Za-z0-9+/]{54}$/.test(value[name]) || Buffer.from(value[name], 'base64').length !== 42)) throw new Error(`Publication ${name} needs a minisign public key (the RW… line).`);
  }
  return { ...value };
}

export function loadPublication(required = publicationLevel()) {
  // HOI_PUBLICATION_FILE builds a test copy with a throwaway signing key (apps/site/scripts/e2e.mjs).
  // A website or release build always reads publication.json: a test key must never ship.
  const override = process.env.HOI_PUBLICATION_FILE;
  if (override && required) throw new Error('HOI_PUBLICATION_FILE is for test builds only; unset it for a website or release build.');
  const file = override ? resolve(override) : new URL('../publication.json', import.meta.url);
  return validatePublication(JSON.parse(readFileSync(file, 'utf8')), required);
}

/** Bake the same trust root into portable installers and Setup's bundled scripts. */
export function configureInstaller(source, config = loadPublication()) {
  const manifest = config.downloads ? `${config.downloads.replace(/\/+$/, '')}/manifest.json` : '';
  const repo = config.repository.replace(/^https:\/\/github.com\//, '').replace(/\/+$/, '');
  // Values are validated as URLs/base64 first; quoting here also covers non-GitHub repositories.
  const sh = value => `'${value.replaceAll("'", "'\\''")}'`;
  const ps = value => `'${value.replaceAll("'", "''")}'`;
  return source.replace(/^DEFAULT_MANIFEST=.*$/m, `DEFAULT_MANIFEST=${sh(manifest)}`)
    .replace(/^DEFAULT_REPO=.*$/m, `DEFAULT_REPO=${sh(repo)}`)
    .replace(/^RELEASE_PUBKEY=.*$/m, `RELEASE_PUBKEY=${sh(config.manifestKey)}`)
    .replace(/(\[string\]\$Manifest = )'[^']*'/, `$1${ps(manifest)}`)
    .replace(/(\[string\]\$PubKey = )'[^']*'/, `$1${ps(config.manifestKey)}`)
    .replace(/(\[string\]\$Repo = )'[^']*'/, `$1${ps(repo)}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  // `--site` checks only what a website deployment needs; everything else checks a release.
  if (process.argv[2] === '--site') {
    const config = loadPublication('site');
    const signed = config.downloads && config.manifestKey;
    console.log(`Website publication configured for ${config.site}${signed ? '' : '; no signed downloads or updates are offered yet'}. No files published.`);
    process.exit(0);
  }
  const config = loadPublication(true);
  if (process.argv[2] === '--installers') {
    const out = process.argv[3];
    if (!out) throw new Error('--installers needs an output directory.');
    mkdirSync(out, { recursive: true });
    for (const name of ['install.sh', 'install.ps1']) writeFileSync(join(out, name), configureInstaller(readFileSync(new URL(`../deploy/install/${name}`, import.meta.url), 'utf8'), config));
  }
  console.log('Publication URLs, contact and public trust keys are configured. No files published.');
}
