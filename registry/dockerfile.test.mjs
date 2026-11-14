// The registry image must hold every file the server loads. A module missing from the Dockerfile's
// COPY lines fails at start with ERR_MODULE_NOT_FOUND, which only a container run would show.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { dirname, join, posix, resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const slash = (p) => p.split('\\').join('/');

/** Files the Dockerfile copies into the image, as image paths relative to /app. */
function copied() {
  const text = readFileSync(join(root, 'registry/Dockerfile'), 'utf8').replace(/\\\r?\n/g, ' ');
  const files = new Set();
  for (const line of text.split(/\r?\n/)) {
    if (!/^COPY\s/.test(line)) continue;
    const args = line.split(/\s+/).slice(1).filter((a) => !a.startsWith('--'));
    const dest = args.pop();
    for (const src of args) files.add(dest.endsWith('/') ? posix.join(dest, posix.basename(src)) : dest);
  }
  return files;
}

/** Relative imports reachable from `entry`, as repository paths. */
function closure(entry, seen = new Set()) {
  if (seen.has(entry)) return seen;
  seen.add(entry);
  const source = readFileSync(join(root, entry), 'utf8');
  for (const m of source.matchAll(/(?:^|\n)\s*(?:import|export)\b[^'"`;]*?from\s*['"](\.[^'"]+)['"]/g)) {
    closure(slash(relative(root, resolve(root, dirname(entry), m[1]))), seen);
  }
  return seen;
}

test('registry image copies every module the server and admin tool import', () => {
  const image = copied();
  // The image path of a repository file: registry/x -> registry/x, scripts/x -> scripts/x.
  const needed = new Set([...closure('registry/server.mjs'), ...closure('registry/admin.mjs')]);
  const missing = [...needed]
    .filter((file) => existsSync(join(root, file)) && !image.has(file));
  assert.deepEqual(missing, []);
});
