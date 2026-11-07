// Emits `sw.js` at build time with a precache list of every file the build writes:
// all bundle chunks and assets (including worker chunks and .wasm) plus everything copied from public/,
// each with the SHA-256 of its final bytes. The worker refuses a file whose bytes differ, which
// covers what SRI attributes cannot: dynamic imports, workers and the wasm. index.html is the one
// exception (network-first, and its own SRI attributes pin the entry script and stylesheet).
// The cache name is derived from the hashes of all those files, so any change ships a new worker.
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative, sep } from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const SKIP = new Set(['_headers', 'sw.js', 'release.json']);

/** @param {string | Uint8Array} data */
const sha = (data) => createHash('sha256').update(data).digest('hex');

/** @param {string} dir @returns {string[]} */
function walk(dir) {
  let out = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out = out.concat(walk(p));
    else out.push(p);
  }
  return out;
}

/** @returns {import('vite').Plugin} */
export default function serviceWorker() {
  let publicDir = '';
  return {
    name: 'hoi-service-worker',
    apply: 'build',
    enforce: 'post',
    configResolved(config) {
      publicDir = config.publicDir;
    },
    // order: 'post' so the hashes (and the cache version) cover the final bytes of every file.
    generateBundle: { order: 'post', handler(_options, bundle) {
      /** @type {Map<string, string>} */
      const files = new Map();
      for (const [fileName, item] of Object.entries(bundle)) {
        if (SKIP.has(fileName) || fileName.endsWith('.map')) continue;
        files.set(fileName, sha(item.type === 'chunk' ? item.code : item.source));
      }
      if (publicDir) {
        let entries = [];
        try { entries = walk(publicDir); } catch { entries = []; }
        for (const abs of entries) {
          const rel = relative(publicDir, abs).split(sep).join('/');
          if (SKIP.has(rel) || rel.startsWith('.')) continue;
          files.set(rel, sha(readFileSync(abs)));
        }
      }
      const list = [...files.keys()].sort();
      const version = sha(list.map((f) => `${f} ${files.get(f)}`).join('\n')).slice(0, 16);
      const manifest = Object.fromEntries(list.map((f) => [f, f === 'index.html' ? null : files.get(f)]));
      const source = readFileSync(join(here, 'service-worker.js'), 'utf8')
        .replace("'%VERSION%'", JSON.stringify(version))
        .replace("'%PRECACHE%'", JSON.stringify(manifest));
      this.emitFile({ type: 'asset', fileName: 'sw.js', source });
    } },
  };
}
