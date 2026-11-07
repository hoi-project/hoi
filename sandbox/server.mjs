import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, dirname, extname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { timingSafeEqual } from 'node:crypto';
import { Suite } from './suite.mjs';
import { ProviderBroker } from './providers.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, '..');
const TYPES = { '.html': 'text/html; charset=utf-8', '.css': 'text/css; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.woff2': 'font/woff2' };
const roles = new Set(['owner', 'member', 'watcher']);
function secretMatches(value, expected) {
  if (typeof value !== 'string' || !expected) return false;
  const left = Buffer.from(value), right = Buffer.from(expected);
  return left.length === right.length && timingSafeEqual(left, right);
}
async function body(request) {
  if (!request.headers['content-type']?.startsWith('application/json')) throw Object.assign(new Error('Send application/json.'), { status: 415 });
  let data = '';
  for await (const chunk of request) {
    data += chunk;
    if (Buffer.byteLength(data) > 131072) throw Object.assign(new Error('Request too large.'), { status: 413 });
  }
  try { return JSON.parse(data || '{}'); } catch { throw new Error('Invalid JSON.'); }
}
export function createControlServer(suite, { token, port = 7070, root = ROOT, broker } = {}) {
  let mutations = Promise.resolve();
  const mutate = (fn) => { const next = mutations.catch(() => {}).then(fn); mutations = next; return next; };
  return http.createServer(async (request, response) => {
    const send = (status, value) => { response.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' }); response.end(JSON.stringify(value)); };
    response.setHeader('Cache-Control', 'no-store');
    response.setHeader('X-Content-Type-Options', 'nosniff');
    response.setHeader('Referrer-Policy', 'no-referrer');
    response.setHeader('Cross-Origin-Resource-Policy', 'same-origin');
    response.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
    try {
      const url = new URL(request.url, `http://localhost:${port}`);
      const path = url.pathname;
      const internal = path.startsWith('/__sandbox/');
      const providerRoute = ['/__sandbox/provider/take', '/__sandbox/provider/complete'].includes(path);
      if (internal) {
        if (!secretMatches(request.headers['x-hoi-sandbox-token'], token) || (!providerRoute && !roles.has(request.headers['x-hoi-sandbox-role']))) return send(403, { error: 'The authenticated local proxy or CLI worker is required.' });
      } else {
        const allowedHosts = new Set([`localhost:${port}`, `127.0.0.1:${port}`, `[::1]:${port}`]);
        if (!allowedHosts.has(request.headers.host)) return send(403, { error: 'Use the local dashboard URL.' });
        if (request.headers.origin && request.headers.origin !== `http://${request.headers.host}`) return send(403, { error: 'Cross-origin control is disabled.' });
        if (request.headers['sec-fetch-site'] === 'cross-site') return send(403, { error: 'Cross-site control is disabled.' });
      }
      if (request.method === 'GET' && path === '/health') return send(suite.state.ready ? 200 : 503, { ready: suite.state.ready, error: suite.state.error });
      if (request.method === 'GET' && path === '/api/state') {
        if (broker) suite.state.providers = broker.status();
        return send(200, suite.state);
      }
      if (providerRoute) {
        if (request.method !== 'POST') return send(405, { error: 'Method not allowed.' });
        if (!broker) return send(503, { error: 'CLI provider bridge is unavailable.' });
        const data = await body(request);
        return send(200, path.endsWith('/take') ? broker.take(data) : broker.complete(data));
      }
      if (request.method === 'GET' && path === '/__sandbox/bootstrap') return send(200, await suite.bootstrap(request.headers['x-hoi-sandbox-role']));
      if (path.startsWith('/api/') || internal) {
        if (!['POST', 'PATCH'].includes(request.method)) return send(405, { error: 'Method not allowed.' });
        if (!internal && request.headers['x-sandbox-control'] !== '1') return send(403, { error: 'The dashboard control header is required.' });
        const data = await body(request);
        const result = await mutate(async () => {
          const agentPath = /^\/api\/agents\/(agent-[a-f0-9]{10})(?:\/(start|stop))?$/.exec(path);
          if (path === '/api/config' && request.method === 'POST') await suite.configure(data);
          else if (path === '/api/agents' && request.method === 'POST') suite.addAgent(data);
          else if (agentPath && request.method === 'PATCH' && !agentPath[2]) suite.editAgent(agentPath[1], data);
          else if (agentPath?.[2] === 'start' && request.method === 'POST') suite.startAgent(agentPath[1]);
          else if (agentPath?.[2] === 'stop' && request.method === 'POST') await suite.stopAgent(agentPath[1]);
          else if (path === '/api/run/start' && request.method === 'POST') suite.startRun();
          else if (path === '/api/run/stop' && request.method === 'POST') await suite.stopRun();
          else if (path === '/__sandbox/link' && request.method === 'POST') {
            if (request.headers['x-hoi-sandbox-role'] !== 'owner') throw Object.assign(new Error('Only the owner persona may link the founder.'), { status: 403 });
            if (typeof data.request !== 'string' || data.request.length > 32000) throw new Error('Invalid device link request.');
            const result = await suite.action('founder', ['link-approve', data.request]);
            return { approval: result.approval };
          } else if (['/__sandbox/click', '/__sandbox/exposure', '/__sandbox/visit'].includes(path) && request.method === 'POST') return suite.participant(request.headers['x-hoi-sandbox-role'], path.split('/').pop(), data);
          else throw Object.assign(new Error('Unknown control route.'), { status: 404 });
          await suite.save();
          return suite.state;
        });
        return send(200, result);
      }
      if (request.method !== 'GET' && request.method !== 'HEAD') return send(405, { error: 'Method not allowed.' });
      const assets = {
        '/': resolve(root, 'sandbox/public/index.html'),
        '/index.html': resolve(root, 'sandbox/public/index.html'),
        '/dashboard.css': resolve(root, 'sandbox/public/dashboard.css'),
        '/dashboard.mjs': resolve(root, 'sandbox/public/dashboard.mjs'),
        '/shared/tokens.css': resolve(root, 'packages/tokens/generated/tokens.css'),
        '/shared/site.css': resolve(root, 'apps/site/src/styles/site.css'),
      };
      let file = assets[path], content;
      if (path === '/shared/fonts.css') {
        file = resolve(root, 'packages/tokens/fonts.css');
        content = (await readFile(file, 'utf8')).replace(/\.\/node_modules\/[^']+\/files\/([^']+)/g, '/fonts/$1');
      } else if (/^\/fonts\/(gabarito-latin(?:-ext)?-wght-normal|geist-mono-latin-(400|500)-normal)\.woff2$/.test(path)) {
        const filename = path.split('/').pop();
        file = resolve(root, 'packages/tokens/node_modules', filename.startsWith('gabarito') ? '@fontsource-variable/gabarito/files' : '@fontsource/geist-mono/files', filename);
      }
      if (!file) return send(404, { error: 'Not found.' });
      content ??= await readFile(file);
      response.writeHead(200, { 'Content-Type': TYPES[extname(file)] || 'application/octet-stream' });
      response.end(request.method === 'HEAD' ? undefined : content);
    } catch (error) {
      if (!response.headersSent) send(error.status || (error.code === 'ENOENT' ? 404 : 400), { error: error.message });
      else response.end();
    }
  });
}

async function main() {
  const token = process.env.HOI_SANDBOX_TOKEN;
  if (!token || token.length < 32) throw new Error('HOI_SANDBOX_TOKEN is missing. Start with node sandbox/launch.mjs.');
  const { Docker } = await import('./docker.mjs');
  const broker = new ProviderBroker();
  const suite = new Suite(new Docker(), { stateFile: process.env.HOI_STATE_FILE || '/data/control.json', providerRequest: (request, signal) => broker.request(request, signal), providerStatus: () => broker.status() });
  const server = createControlServer(suite, { token, broker });
  server.listen(7070, '0.0.0.0', () => console.log('hoi sandbox dashboard listening on :7070; preparing real identities…'));
  try { await suite.init(); console.log('hoi sandbox ready.'); } catch (error) { suite.state.error = error.message; console.error(error); }
  let refreshing = false;
  const timer = setInterval(async () => {
    if (refreshing || suite.shutdown) return;
    refreshing = true;
    try { if (!suite.state.ready) await suite.init(); else await suite.refresh(); }
    catch (error) { suite.state.error = error.message; }
    finally { refreshing = false; }
  }, 10000);
  const stop = async () => { suite.shutdown = true; clearInterval(timer); await suite.stopRun(); broker.close(); await suite.save(); server.close(); };
  process.on('SIGTERM', stop); process.on('SIGINT', stop);
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) main().catch(error => { console.error(error.message); process.exitCode = 1; });
