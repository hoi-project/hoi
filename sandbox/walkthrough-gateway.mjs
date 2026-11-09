// Only this observation proxy joins the ingress network. Protocol clients and services stay
// on the internal network; this fixed upstream is never chosen from request data.
import { createServer, request as forward } from 'node:http';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export function createWalkthroughGateway({ upstream = 'http://walkthrough:7080', port = 7080 } = {}) {
  const destination = new URL(upstream);
  return createServer({ requestTimeout: 30000, headersTimeout: 10000 }, (request, response) => {
    const origin = `http://${request.headers.host}`;
    const hosts = [`localhost:${port}`, `127.0.0.1:${port}`, `[::1]:${port}`];
    if (!hosts.includes(request.headers.host) || !request.url.startsWith('/') || request.url.startsWith('//')
      || (request.headers.origin && request.headers.origin !== origin)) {
      response.writeHead(403, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      response.end('{"error":"Use the localhost walkthrough."}');
      return;
    }
    const headers = { ...request.headers };
    for (const name of ['connection', 'proxy-authorization', 'proxy-connection', 'upgrade']) delete headers[name];
    // request.url is only the path; it cannot replace the fixed upstream host or port.
    const proxy = forward({ hostname: destination.hostname, port: destination.port, method: request.method, path: request.url, headers }, result => {
      response.writeHead(result.statusCode, result.headers);
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
