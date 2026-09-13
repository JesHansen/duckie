// Optional deterministic local fixture server. No dependencies or outbound connections.
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { gzipSync } from 'node:zlib';

const port = Number(process.env.DUCKIE_TEST_PORT || 8787);
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${port}`);
  if (url.pathname.endsWith('/openapi.json')) {
    if (url.pathname.startsWith('/protected/') && req.headers.authorization !== 'Bearer duckie-local-demo') {
      res.writeHead(401, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ error: 'Supply the local demo bearer token' })); return;
    }
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(await readFile(new URL('../examples/openapi.json', import.meta.url))); return;
  }
  if (url.pathname === '/redirect') {
    res.writeHead(302, { location: '/echo', 'content-length': '0' }); res.end(); return;
  }
  if (url.pathname === '/slow') {
    const timer = setTimeout(() => { res.writeHead(200, { 'content-type': 'application/json' }); res.end('{"finished":true}'); }, 10000);
    res.on('close', () => clearTimeout(timer)); return;
  }
  if (url.pathname === '/large') {
    const bytes = Math.min(60 * 1024 * 1024, Math.max(1, Number(url.searchParams.get('bytes') || 4 * 1024 * 1024)));
    // `lines=true` breaks the payload into 80-column rows; single-line and multi-line bodies
    // cost very different amounts to lay out, and the benchmark protocol wants both shapes.
    const body = Buffer.alloc(bytes, 97);
    if (url.searchParams.get('lines') === 'true') for (let i = 79; i < bytes; i += 80) body[i] = 10;
    const gzip = url.searchParams.get('gzip') === 'true';
    res.writeHead(200, { 'content-type': 'text/plain', ...(gzip ? { 'content-encoding': 'gzip' } : {}) });
    res.end(gzip ? gzipSync(body) : body); return;
  }
  const chunks = []; let size = 0;
  for await (const chunk of req) { size += chunk.length; if (size > 20 * 1024 * 1024) { res.writeHead(413); res.end(); return; } chunks.push(chunk); }
  const code = Number(url.pathname.match(/^\/status\/(\d{3})$/)?.[1] || 200);
  const body = { method: req.method, path: url.pathname, query: [...url.searchParams], headers: req.headers, body: Buffer.concat(chunks).toString('utf8') };
  res.writeHead(code, { 'content-type': 'application/json', 'x-duplicate': ['first', 'second'] });
  res.end(JSON.stringify(body, null, 2));
});
server.listen(port, '127.0.0.1', () => console.log(`Duckie local fixtures: http://127.0.0.1:${port}`));
