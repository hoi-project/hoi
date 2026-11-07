// Three browser origins keep the owner, another member and a watcher independent. Only this
// proxy knows the controller token; neither the token nor an owner key is sent to browser JS.
import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { pathToFileURL } from 'node:url';

const MIME = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.json': 'application/json', '.wasm': 'application/wasm', '.woff2': 'font/woff2', '.svg': 'image/svg+xml', '.png': 'image/png', '.webmanifest': 'application/manifest+json' };
const ROLES = { 7071: 'owner', 7072: 'member', 7073: 'watcher' };
const ROUTES = new Map([
  ['/__sandbox/bootstrap', 'GET'], ['/__sandbox/link', 'POST'], ['/__sandbox/click', 'POST'],
  ['/__sandbox/visit', 'POST'], ['/__sandbox/exposure', 'POST'],
]);

function reply(response, status, value) {
  response.writeHead(status, { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store', 'x-content-type-options': 'nosniff' });
  response.end(JSON.stringify(value));
}

export function validHost(host, port) {
  return [`127.0.0.1:${port}`, `localhost:${port}`, `[::1]:${port}`].includes(host?.toLowerCase());
}

async function body(request) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > 16384) throw new Error('Request too large.');
    chunks.push(chunk);
  }
  const value = Buffer.concat(chunks).toString('utf8');
  const parsed = JSON.parse(value);
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('Expected an object.');
  return value;
}

export function createAppHandler({ root = '/app/dist', controller = 'http://dashboard:7070', token, role, port }) {
  const directory = resolve(root);
  if (!token || token.length < 24) throw new Error('HOI_SANDBOX_TOKEN must contain at least 24 characters.');
  if (ROLES[port] !== role) throw new Error('The app role must match its dedicated port.');
  return async (request, response) => {
    if (!validHost(request.headers.host, port)) return reply(response, 403, { error: 'Open this sandbox on localhost.' });
    let url;
    try { url = new URL(request.url, `http://${request.headers.host}`); }
    catch { return reply(response, 400, { error: 'Invalid URL.' }); }
    if (url.origin !== `http://${request.headers.host}`) return reply(response, 400, { error: 'Invalid origin.' });
    if (url.pathname.startsWith('/__sandbox/')) {
      if (ROUTES.get(url.pathname) !== request.method) return reply(response, 404, { error: 'Unknown sandbox action.' });
      if (url.pathname === '/__sandbox/link' && role !== 'owner') return reply(response, 403, { error: 'Only the owner browser can link to the owner.' });
      if (request.method === 'POST' && (request.headers.origin !== url.origin || request.headers['content-type']?.split(';')[0].trim() !== 'application/json')) {
        return reply(response, 403, { error: 'Sandbox actions must come from this app.' });
      }
      let payload;
      if (request.method === 'POST') {
        try { payload = await body(request); }
        catch { return reply(response, 400, { error: 'Expected a small JSON object.' }); }
      }
      try {
        const upstream = await fetch(new URL(url.pathname, controller), {
          method: request.method,
          headers: { 'content-type': 'application/json', 'x-hoi-sandbox-token': token, 'x-hoi-sandbox-role': role },
          body: payload, signal: AbortSignal.timeout(60000), redirect: 'error',
        });
        response.writeHead(upstream.status, { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store', 'x-content-type-options': 'nosniff' });
        response.end(await upstream.text());
      } catch {
        reply(response, 503, { error: 'The sandbox dashboard is not ready. Wait for startup, then reload.' });
      }
      return;
    }
    if (request.method !== 'GET' && request.method !== 'HEAD') return reply(response, 405, { error: 'Method not allowed.' });
    let pathname;
    try { pathname = decodeURIComponent(url.pathname); }
    catch { return reply(response, 400, { error: 'Invalid path.' }); }
    if (pathname === '/app') {
      response.writeHead(308, { location: '/app/', 'cache-control': 'no-store' });
      response.end();
      return;
    }
    if (pathname.startsWith('/app/')) pathname = pathname.slice(4);
    if (pathname.includes('\0') || pathname.includes('\\')) return reply(response, 400, { error: 'Invalid path.' });
    const filename = resolve(directory, `.${pathname === '/' ? '/index.html' : pathname}`);
    if (!filename.startsWith(directory + sep)) return reply(response, 403, { error: 'Invalid path.' });
    try {
      if (!(await stat(filename)).isFile()) return reply(response, 404, { error: 'File not found.' });
      const bytes = await readFile(filename);
      response.writeHead(200, {
        'content-type': MIME[extname(filename)] ?? 'application/octet-stream',
        'cache-control': 'no-store', 'x-content-type-options': 'nosniff',
        'referrer-policy': 'no-referrer', 'cross-origin-opener-policy': 'same-origin',
        'cross-origin-resource-policy': 'same-origin', 'x-frame-options': 'DENY',
      });
      response.end(request.method === 'HEAD' ? undefined : bytes);
    } catch { reply(response, 404, { error: 'File not found.' }); }
  };
}

export function startAppServers(options = {}) {
  const servers = Object.entries(ROLES).map(([port, role]) => {
    const server = createServer(createAppHandler({
      root: process.env.HOI_APP_DIR || '/app/dist',
      controller: process.env.HOI_SANDBOX_CONTROLLER || 'http://dashboard:7070',
      token: process.env.HOI_SANDBOX_TOKEN,
      ...options, port: Number(port), role,
    }));
    server.listen(Number(port), '0.0.0.0', () => console.log(`Sandbox ${role}: http://localhost:${port}/app/`));
    return server;
  });
  const close = () => { for (const server of servers) server.close(); };
  process.once('SIGTERM', close);
  process.once('SIGINT', close);
  return servers;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) startAppServers();
