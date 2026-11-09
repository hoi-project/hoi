// Only this observation proxy joins the ingress network. Protocol clients and services stay
// on the internal network; this fixed upstream is never chosen from request data.
import { createServer, request as forward } from 'node:http';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const routes = [
  '/', '/walkthrough.html', '/walkthrough.css', '/walkthrough.mjs', '/walkthrough-story.mjs', '/walkthrough-player.mjs',
  '/shared/tokens.css', '/shared/site.css', '/shared/fonts.css',
  '/fonts/gabarito-latin-wght-normal.woff2', '/fonts/gabarito-latin-ext-wght-normal.woff2',
  '/fonts/geist-mono-latin-400-normal.woff2', '/fonts/geist-mono-latin-500-normal.woff2',
  '/health', '/api/state', '/api/publish', '/api/reset', '/api/journey/start', '/api/journey/next',
  '/api/screenshot/alice', '/api/screenshot/bob',
];

function forwardPath(target) {
  const [pathname, query] = target.split('?');
  const route = routes.find(candidate => candidate === pathname);
  if (!route || !route.startsWith('/api/screenshot/')) return route;
  const visibility = new URLSearchParams(query).get('visibility');
  if (visibility === 'public') return `${route}?visibility=public`;
  if (visibility === 'private') return `${route}?visibility=private`;
  return visibility === null ? route : undefined;
}

export function createWalkthroughGateway({ upstreamHost = 'walkthrough', upstreamPort = 7080, port = 7080 } = {}) {
  // Plain HTTP stays inside the isolated Docker network, or on loopback in local tests.
  // Configuration cannot point this observation proxy at a remote service.
  const hostname = ['walkthrough', 'localhost', '127.0.0.1', '::1'].find(host => host === upstreamHost);
  if (!hostname || !Number.isInteger(upstreamPort) || upstreamPort < 1 || upstreamPort > 65535) throw new Error('Use a local walkthrough upstream.');
  return createServer({ requestTimeout: 30000, headersTimeout: 10000 }, (request, response) => {
    const origin = `http://${request.headers.host}`;
    const hosts = [`localhost:${port}`, `127.0.0.1:${port}`, `[::1]:${port}`];
    const path = forwardPath(request.url);
    if (!hosts.includes(request.headers.host) || !path
      || (request.headers.origin && request.headers.origin !== origin)) {
      response.writeHead(403, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      response.end('{"error":"Use the localhost walkthrough."}');
      return;
    }
    const headers = { ...request.headers };
    for (const name of ['connection', 'proxy-authorization', 'proxy-connection', 'upgrade']) delete headers[name];
    // Only fixed walkthrough paths reach the local upstream. Its arbitrary header names,
    // redirects and security policy must not become the browser's response headers.
    const proxy = forward({ hostname, port: upstreamPort, method: request.method, path, headers }, result => {
      response.setHeader('Content-Type', result.headers['content-type'] || 'application/octet-stream');
      if (result.headers['content-length']) response.setHeader('Content-Length', result.headers['content-length']);
      response.setHeader('Cache-Control', 'no-store');
      response.setHeader('X-Content-Type-Options', 'nosniff');
      response.setHeader('Referrer-Policy', 'no-referrer');
      response.setHeader('Cross-Origin-Resource-Policy', 'same-origin');
      response.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self'; connect-src 'self'; frame-ancestors 'none'; object-src 'none'; base-uri 'none'; form-action 'self'");
      response.writeHead(result.statusCode);
      // A partial screenshot/JSON response cannot be repaired with a second error body.
      result.on('error', () => response.destroy());
      result.on('aborted', () => response.destroy());
      result.pipe(response);
    });
    proxy.setTimeout(600_000, () => proxy.destroy());
    proxy.on('error', () => {
      if (response.headersSent) { response.destroy(); return; }
      response.writeHead(503, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      response.end('{"error":"The local network is starting. Try again shortly."}');
    });
    request.once('aborted', () => proxy.destroy());
    response.once('close', () => { if (!response.writableFinished) proxy.destroy(); });
    request.pipe(proxy);
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const server = createWalkthroughGateway();
  server.listen(7080, '0.0.0.0');
  const close = () => { server.closeAllConnections(); server.close(); };
  process.once('SIGTERM', close);
  process.once('SIGINT', close);
}
