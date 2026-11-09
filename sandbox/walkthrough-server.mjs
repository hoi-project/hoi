// Local observation surface. This module is never imported by a production service or app.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, extname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const TYPES = { '.html': 'text/html; charset=utf-8', '.css': 'text/css; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.woff2': 'font/woff2' };
const failure = (message, status = 400) => Object.assign(new Error(message), { status });

export function validatePost(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || Object.keys(value).some(key => !['author', 'visibility', 'title', 'text'].includes(key))
    || !['alice', 'bob'].includes(value.author) || !['public', 'private'].includes(value.visibility)
    || typeof value.title !== 'string' || !value.title.trim() || value.title.length > 120
    || typeof value.text !== 'string' || !value.text.trim() || value.text.length > 4000) {
    throw failure('Choose a seeded user and community, with a title (1–120 characters) and message (1–4000 characters).');
  }
  return { author: value.author, visibility: value.visibility, title: value.title, text: value.text };
}

export function validateJourneyStep(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || Object.keys(value).some(key => !['journeyId', 'stage'].includes(key))
    || typeof value.journeyId !== 'string' || !/^[a-zA-Z0-9_-]{1,128}$/.test(value.journeyId)
    || !['draft', 'seal', 'store', 'download', 'open'].includes(value.stage)) {
    throw failure('Send the journey and stage currently displayed by the walkthrough.');
  }
  return { journeyId: value.journeyId, stage: value.stage };
}

async function readBody(request) {
  if (request.headers['content-type']?.split(';')[0].trim() !== 'application/json') throw failure('Send application/json.', 415);
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > 16384) throw failure('Request too large.', 413);
    chunks.push(chunk);
  }
  try {
    const data = JSON.parse(Buffer.concat(chunks).toString('utf8'));
    if (!data || typeof data !== 'object' || Array.isArray(data)) throw new Error();
    return data;
  } catch { throw failure('Send a JSON object.'); }
}

export function createWalkthroughServer(network, { port = 7080, root = ROOT } = {}) {
  let busy = false;
  const server = createServer(async (request, response) => {
    const send = (status, value) => { response.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' }); response.end(JSON.stringify(value)); };
    response.setHeader('Cache-Control', 'no-store');
    response.setHeader('X-Content-Type-Options', 'nosniff');
    response.setHeader('Referrer-Policy', 'no-referrer');
    response.setHeader('Cross-Origin-Resource-Policy', 'same-origin');
    response.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self'; connect-src 'self'; frame-ancestors 'none'; object-src 'none'; base-uri 'none'; form-action 'self'");
    try {
      const actualPort = port || server.address()?.port;
      const hosts = [`localhost:${actualPort}`, `127.0.0.1:${actualPort}`, `[::1]:${actualPort}`];
      if (!hosts.includes(request.headers.host)) return send(403, { error: 'Open the walkthrough on localhost.' });
      const origin = `http://${request.headers.host}`;
      if ((request.headers.origin && request.headers.origin !== origin) || request.headers['sec-fetch-site'] === 'cross-site') return send(403, { error: 'Cross-origin control is disabled.' });
      const url = new URL(request.url, origin);
      if (url.origin !== origin) return send(403, { error: 'Invalid origin.' });
      if (request.method === 'GET' && url.pathname === '/health') return send(200, { running: true });
      if (request.method === 'GET' && url.pathname === '/api/state') {
        const state = await network.snapshot();
        return send(200, { ...state, busy: busy || state.status === 'starting' });
      }
      const screenshot = /^\/api\/screenshot\/(alice|bob)$/.exec(url.pathname);
      if (request.method === 'GET' && screenshot) {
        if (busy || (await network.snapshot()).status === 'starting') return send(409, { error: 'The view is changing. Refresh after this step finishes.' });
        const visibility = url.searchParams.get('visibility');
        if (visibility !== null && !['public', 'private'].includes(visibility)) throw failure('Choose a public or private community view.');
        const bytes = await network.screenshot(screenshot[1], visibility || undefined);
        response.writeHead(200, { 'Content-Type': 'image/png' });
        return response.end(bytes);
      }
      if (['/api/publish', '/api/reset', '/api/journey/start', '/api/journey/next'].includes(url.pathname)) {
        if (request.method !== 'POST') return send(405, { error: 'Method not allowed.' });
        // A custom header plus same-origin JSON prevents another page driving the local clients.
        if (request.headers.origin !== origin || request.headers['x-sandbox-control'] !== '1') return send(403, { error: 'Use the walkthrough controls.' });
        const body = await readBody(request);
        const post = ['/api/publish', '/api/journey/start'].includes(url.pathname) ? validatePost(body) : null;
        const step = url.pathname === '/api/journey/next' ? validateJourneyStep(body) : null;
        if (!post && !step && Object.keys(body).length) throw failure('This action takes no parameters.');
        const state = await network.snapshot();
        // Startup owns the same clients and stores as reset; never tear them down mid-seed.
        if (busy || state.status === 'starting') return send(409, { error: 'A walkthrough step is already running.' });
        busy = true;
        try {
          if (url.pathname === '/api/journey/start') await network.startJourney(post);
          else if (url.pathname === '/api/journey/next') await network.nextJourney(step);
          else if (post) await network.publish(post);
          else await network.reset();
          return send(200, { ...await network.snapshot(), busy: false });
        } catch (error) {
          if (error.code === 'WALKTHROUGH_SEQUENCE') return send(409, { error: 'This journey step is no longer available. Refresh the walkthrough.' });
          if (error.code === 'WALKTHROUGH_DUPLICATE') return send(422, { error: 'You already sent this message. Change its title or text and prepare it again, or reset the demonstration to repeat it.' });
          return send(500, { error: 'This step did not finish. Inspect the walkthrough status, then retry or reset.' });
        } finally { busy = false; }
      }
      if (!['GET', 'HEAD'].includes(request.method)) return send(405, { error: 'Method not allowed.' });
      const assets = {
        '/': 'sandbox/public/walkthrough.html', '/walkthrough.html': 'sandbox/public/walkthrough.html',
        '/walkthrough.css': 'sandbox/public/walkthrough.css', '/walkthrough.mjs': 'sandbox/public/walkthrough.mjs',
        '/walkthrough-story.mjs': 'sandbox/public/walkthrough-story.mjs', '/walkthrough-player.mjs': 'sandbox/public/walkthrough-player.mjs',
        '/shared/tokens.css': 'packages/tokens/generated/tokens.css', '/shared/site.css': 'apps/site/src/styles/site.css',
      };
      let file = assets[url.pathname], bytes;
      if (url.pathname === '/shared/fonts.css') {
        file = 'packages/tokens/fonts.css';
        bytes = (await readFile(resolve(root, file), 'utf8')).replace(/\.\/node_modules\/[^']+\/files\/([^']+)/g, '/fonts/$1');
      } else if (/^\/fonts\/(gabarito-latin(?:-ext)?-wght-normal|geist-mono-latin-(400|500)-normal)\.woff2$/.test(url.pathname)) {
        const name = url.pathname.split('/').pop();
        file = `packages/tokens/node_modules/${name.startsWith('gabarito') ? '@fontsource-variable/gabarito' : '@fontsource/geist-mono'}/files/${name}`;
      }
      if (!file) return send(404, { error: 'Not found.' });
      bytes ??= await readFile(resolve(root, file));
      response.writeHead(200, { 'Content-Type': TYPES[extname(file)] || 'application/octet-stream' });
      response.end(request.method === 'HEAD' ? undefined : bytes);
    } catch (error) {
      if (!response.headersSent) send(error.status || (error.code === 'ENOENT' ? 404 : 503), { error: error.status ? error.message : 'The walkthrough is not ready. Wait for startup or reset.' });
      else response.end();
    }
  });
  server.requestTimeout = 30000;
  return server;
}

async function main() {
  const { createWalkthroughNetwork } = await import('./walkthrough-network.mjs');
  const network = createWalkthroughNetwork({ stateDir: process.env.HOI_WALKTHROUGH_DATA || resolve(ROOT, 'sandbox/.state/walkthrough'), nodeCommand: process.env.HOI_NODE_BIN });
  const server = createWalkthroughServer(network);
  const host = process.env.HOI_WALKTHROUGH_CONTAINER === '1' ? '0.0.0.0' : '127.0.0.1';
  server.listen(7080, host, () => console.log('Network walkthrough: http://localhost:7080'));
  const close = async () => { server.closeAllConnections(); server.close(); await network.close(); };
  process.once('SIGTERM', () => { void close(); });
  process.once('SIGINT', () => { void close(); });
  try { await network.start(); }
  catch { console.error('Network setup failed. The walkthrough shows which step needs attention.'); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await main();
