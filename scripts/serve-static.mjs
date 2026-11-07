// Serves the assembled dist/ the way a plain static host (GitHub Pages) does: files only, GET and
// HEAD only, `dir/` serves `dir/index.html`, `dir` redirects to `dir/`, `page` falls back to
// `page.html`, anything else is 404 with 404.html. No custom headers: whatever the pages need
// travels inside them (the CSP and referrer <meta> tags).
//
//   node scripts/serve-static.mjs [--port 8787] [--dir dist]
import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const TYPES = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.json': 'application/json', '.webmanifest': 'application/manifest+json',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon', '.woff2': 'font/woff2', '.wasm': 'application/wasm',
  '.txt': 'text/plain; charset=utf-8', '.sh': 'application/x-sh', '.ps1': 'text/plain; charset=utf-8', '.sig': 'application/octet-stream',
};

async function file(path) {
  try { return (await stat(path)).isFile() ? path : null; } catch { return null; }
}

async function dir(path) {
  try { return (await stat(path)).isDirectory(); } catch { return false; }
}

/** Starts the server; resolves to { origin, close }. Port 0 picks a free port. */
export async function serveStatic({ root = 'dist', port = 0, host = '127.0.0.1' } = {}) {
  const base = resolve(root);
  const server = createServer(async (req, res) => {
    if (req.method !== 'GET' && req.method !== 'HEAD') {
      res.writeHead(405, { Allow: 'GET, HEAD' }).end();
      return;
    }
    let pathname;
    try { pathname = decodeURIComponent(new URL(req.url, 'http://x').pathname); } catch { res.writeHead(400).end(); return; }
    const target = resolve(base, '.' + pathname);
    if (target !== base && !target.startsWith(base + sep)) { res.writeHead(404).end(); return; }
    let found = null;
    if (pathname.endsWith('/')) found = await file(join(target, 'index.html'));
    else if (await dir(target)) { res.writeHead(301, { Location: pathname + '/' }).end(); return; }
    else found = (await file(target)) ?? (await file(target + '.html'));
    const status = found ? 200 : 404;
    found ??= await file(join(base, '404.html'));
    const body = found ? await readFile(found) : Buffer.from('Not found');
    res.writeHead(status, { 'Content-Type': (found && TYPES[extname(found)]) || 'application/octet-stream', 'Content-Length': body.length });
    res.end(req.method === 'HEAD' ? undefined : body);
  });
  await new Promise((ok, fail) => server.once('error', fail).listen(port, host, ok));
  const { port: bound } = server.address();
  return { origin: `http://${host}:${bound}`, close: () => new Promise(ok => server.close(ok)) };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const arg = (name, fallback) => { const i = process.argv.indexOf(name); return i > 0 ? process.argv[i + 1] : fallback; };
  const { origin } = await serveStatic({ root: arg('--dir', 'dist'), port: Number(arg('--port', 8787)) });
  console.log(`Serving ${arg('--dir', 'dist')} at ${origin}`);
}
