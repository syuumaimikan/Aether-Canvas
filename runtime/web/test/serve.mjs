// A minimal static file server for the web player's tests and demo.
//
//   node runtime/web/test/serve.mjs [port]     then open http://localhost:8080/

import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, normalize, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.json': 'application/json',
  '.png': 'image/png',
  '.wasm': 'application/wasm',
  '.css': 'text/css',
};

/** Serve `root` on `port` (0 picks a free one). Resolves to the server. */
export function serve(root, port = 0) {
  const base = resolve(root);
  const server = createServer(async (request, response) => {
    const path = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    const file = normalize(resolve(base, '.' + (path.endsWith('/') ? path + 'index.html' : path)));
    if (file !== base && !file.startsWith(base + sep)) {
      response.writeHead(403).end();
      return;
    }
    try {
      const body = await readFile(file);
      response.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' });
      response.end(body);
    } catch {
      response.writeHead(404).end('not found');
    }
  });
  return new Promise((done) => server.listen(port, '127.0.0.1', () => done(server)));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('../', import.meta.url));
  const server = await serve(root, Number(process.argv[2] ?? 8080));
  console.log(`serving ${root} at http://localhost:${server.address().port}/`);
}
