// Checks the release list the Website workflow puts on the site: manifest.json must verify against
// publication.json's manifestKey before it replaces the placeholder.
// Usage: node scripts/release-site-list.mjs <dir with manifest.json and manifest.json.minisig>
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadPublication } from './publication.mjs';
import { parseManifest, verifyMinisign } from './release-verify.mjs';

export function checkSiteList(dir, key = loadPublication().manifestKey) {
  const bytes = readFileSync(join(dir, 'manifest.json'));
  verifyMinisign(bytes, readFileSync(join(dir, 'manifest.json.minisig'), 'utf8'), key);
  return parseManifest(bytes);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [dir] = process.argv.slice(2);
  if (!dir) throw new Error('usage: release-site-list.mjs <dir>');
  console.log(`Release list for ${checkSiteList(dir).latest} verified against manifestKey.`);
}
