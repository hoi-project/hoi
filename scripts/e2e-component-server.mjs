import { resolve } from 'node:path';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';

/** Serve an authored component fixture through the app's Vite configuration. */
export async function startComponentServer(path, html) {
  const server = await createServer({
    root: resolve(import.meta.dirname, '../apps/web'),
    logLevel: 'error',
    server: { host: '127.0.0.1', port: 0, hmr: false },
    plugins: [{
      name: 'component-test-page',
      configureServer(server) {
        server.middlewares.use(path, async (_req, res, next) => {
          try {
            const document = await server.transformIndexHtml(path, html);
            res.setHeader('Content-Type', 'text/html');
            res.end(document);
          } catch (error) {
            next(error);
          }
        });
      },
    }],
  });
  await server.listen();
  return { server, origin: `http://127.0.0.1:${server.httpServer.address().port}` };
}
