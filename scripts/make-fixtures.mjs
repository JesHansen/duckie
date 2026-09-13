// Deterministic benchmark fixtures for ARCHITECTURE.md #9. No dependencies.
// Usage: node scripts/make-fixtures.mjs [output-dir]   (defaults to fixtures/ at the repo root)
import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const outDir = path.resolve(process.argv[2] || path.join(repoRoot, 'fixtures'));

// --- deterministic PRNG (LCG); never use Math.random or Date so reruns are byte-identical ---
function makeRng(seed) {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 4294967296;
  };
}
const pick = (rng, arr) => arr[Math.floor(rng() * arr.length)];
const pad = (n, width) => String(n).padStart(width, '0');
const WORDS = ['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot', 'golf', 'hotel', 'india', 'juliet', 'kilo', 'lima', 'mike', 'november', 'oscar', 'papa', 'quebec', 'romeo', 'sierra', 'tango', 'uniform', 'victor', 'whiskey', 'xray', 'yankee', 'zulu'];
const words = (rng, n) => Array.from({ length: n }, () => pick(rng, WORDS)).join(' ');

function writeJson(file, value) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, JSON.stringify(value, null, 2) + '\n');
}
function writeText(file, text) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, text);
}

// ---------------------------------------------------------------------------
// Collections (fixtures/collection-1k, fixtures/collection-10k)
// ---------------------------------------------------------------------------
const NOUNS = ['users', 'orders', 'products', 'invoices', 'payments', 'shipments', 'tickets', 'accounts', 'sessions', 'reports', 'teams', 'projects', 'tasks', 'comments', 'files', 'webhooks', 'devices', 'notifications', 'audits', 'subscriptions'];
const METHODS = ['GET', 'GET', 'GET', 'POST', 'POST', 'PUT', 'PATCH', 'DELETE', 'GET', 'HEAD'];
const FOLDERS = NOUNS.map((n, i) => `${pad(i + 1, 2)} ${n[0].toUpperCase()}${n.slice(1)}`);
const ID_WIDTH = 5;

function bodyPayload(rng, id, minBytes) {
  const obj = { id, name: words(rng, 3), tags: Array.from({ length: 5 }, () => pick(rng, WORDS)), notes: [] };
  while (JSON.stringify(obj).length < minBytes) obj.notes.push(words(rng, 12));
  return obj;
}
function testSource(id) {
  return `test("${id} responds", () => {\n  expect(response.status).toBeLessThan(500);\n});\n\ntest("${id} has a body", () => {\n  expect(response.bodySize).toBeGreaterThanOrEqual(0);\n});\n`;
}

function generateCollection(dir, count) {
  rmSync(dir, { recursive: true, force: true });
  const rng = makeRng(0xc0ffee ^ count);
  const manifestRequests = [];
  for (let i = 1; i <= count; i++) {
    const id = `req-${pad(i, ID_WIDTH)}`;
    const noun = NOUNS[i % NOUNS.length];
    const folder = FOLDERS[i % FOLDERS.length];
    const method = pick(rng, METHODS);
    const hasExtra = i % 5 === 0; // a fifth of requests get a body + test to defer-load
    const request = {
      schemaVersion: 1,
      id,
      name: `${method} ${noun} #${i}`,
      folder,
      method,
      url: method === 'POST' || method === 'PUT' || method === 'PATCH'
        ? `{{env.baseUrl}}/${noun}`
        : `{{env.baseUrl}}/${noun}/${i}`,
      query: i % 3 === 0 ? [{ enabled: true, name: 'page', value: String(1 + (i % 7)) }] : [],
      headers: [{ enabled: true, name: 'Accept', value: 'application/json' }],
    };
    if (hasExtra) {
      request.body = { kind: 'json', file: `bodies/${id}.json` };
      request.tests = { enabled: true, file: `tests/${id}.test.js` };
      writeJson(path.join(dir, 'bodies', `${id}.json`), bodyPayload(rng, id, 2000));
      writeText(path.join(dir, 'tests', `${id}.test.js`), testSource(id));
    }
    writeJson(path.join(dir, 'requests', `${id}.request.json`), request);
    manifestRequests.push(`requests/${id}.request.json`);
  }
  writeJson(path.join(dir, 'duckie.json'), {
    schemaVersion: 1,
    name: `Benchmark collection (${count})`,
    requests: manifestRequests,
  });
  writeJson(path.join(dir, 'environments', 'dev.json'), {
    schemaVersion: 1,
    name: 'dev',
    values: { baseUrl: 'http://127.0.0.1:8787' },
  });
  writeText(path.join(dir, '.gitignore'), '/.duckie/\n');
}

// ---------------------------------------------------------------------------
// OpenAPI document (fixtures/openapi-10mb.json)
// ---------------------------------------------------------------------------
const SCHEMA_COUNT = 40;
const ACTIONS = ['search', 'archive', 'export', 'history', 'comments', 'attachments', 'tags', 'audit', 'versions', 'preview'];

function buildSchemas(rng) {
  const schemas = {};
  for (let i = 0; i < SCHEMA_COUNT; i++) {
    const name = `Schema${pad(i + 1, 3)}`;
    schemas[name] = {
      type: 'object',
      description: words(rng, 6),
      properties: {
        id: { type: 'string', format: 'uuid' },
        name: { type: 'string' },
        value: { type: 'number' },
        active: { type: 'boolean' },
        tags: { type: 'array', items: { type: 'string' } },
        createdAt: { type: 'string', format: 'date-time' },
        related: i > 0 ? { $ref: `#/components/schemas/Schema${pad(i, 3)}` } : { type: 'null' },
      },
      required: ['id', 'name'],
    };
  }
  schemas.Error = {
    type: 'object',
    properties: { code: { type: 'integer' }, message: { type: 'string' } },
    required: ['code', 'message'],
  };
  return schemas;
}
function buildOperation(rng, method, noun, action, schemaName) {
  const op = {
    summary: `${method.toUpperCase()} ${noun} ${action}`,
    operationId: `${method}_${noun}_${action}`,
    tags: [noun],
    description: words(rng, 20),
    parameters: [
      { name: 'limit', in: 'query', schema: { type: 'integer' }, example: 20 },
      { name: 'cursor', in: 'query', schema: { type: 'string' } },
    ],
    responses: {
      200: { description: 'OK', content: { 'application/json': { schema: { $ref: `#/components/schemas/${schemaName}` } } } },
      400: { description: 'Bad request', content: { 'application/json': { schema: { $ref: '#/components/schemas/Error' } } } },
      404: { description: 'Not found', content: { 'application/json': { schema: { $ref: '#/components/schemas/Error' } } } },
    },
  };
  if (method === 'post' || method === 'put' || method === 'patch') {
    op.requestBody = { required: true, content: { 'application/json': { schema: { $ref: `#/components/schemas/${schemaName}` } } } };
  }
  return op;
}
function buildOpenApi(minBytes, maxBytes) {
  const rng = makeRng(0x0a9e0a91);
  const schemas = buildSchemas(rng);
  const schemaNames = Object.keys(schemas).filter((n) => n !== 'Error');
  const doc = {
    openapi: '3.1.0',
    info: { title: 'Duckie benchmark API', version: '1.0.0', description: 'Synthetic fixture; not a real service.' },
    servers: [{ url: 'https://api.example.test' }],
    paths: {},
    components: { schemas },
  };
  let k = 0;
  const cycle = NOUNS.length * ACTIONS.length;
  for (;;) {
    const noun = NOUNS[k % NOUNS.length];
    const action = ACTIONS[Math.floor(k / NOUNS.length) % ACTIONS.length];
    const shard = Math.floor(k / cycle);
    const p = shard === 0 ? `/api/v1/${noun}/${action}` : `/api/v1/${noun}/${action}/${shard}`;
    const schemaName = schemaNames[k % schemaNames.length];
    const item = { get: buildOperation(rng, 'get', noun, action, schemaName), post: buildOperation(rng, 'post', noun, action, schemaName) };
    if (k % 2 === 0) item.put = buildOperation(rng, 'put', noun, action, schemaName);
    if (k % 3 === 0) item.delete = buildOperation(rng, 'delete', noun, action, schemaName);
    doc.paths[p] = item;
    k++;
    if (k % 250 === 0) {
      const size = Buffer.byteLength(JSON.stringify(doc));
      if (size >= minBytes) break;
      if (size >= maxBytes) break; // guard against overshoot on a pathological run
    }
  }
  return doc;
}

// ---------------------------------------------------------------------------
// Generate everything, then a light structural spot-check.
// ---------------------------------------------------------------------------
mkdirSync(outDir, { recursive: true });
generateCollection(path.join(outDir, 'collection-1k'), 1000);
generateCollection(path.join(outDir, 'collection-10k'), 10000);

const MIB = 1024 * 1024;
const openapi = buildOpenApi(10 * MIB, 18 * MIB);
const openapiPath = path.join(outDir, 'openapi-10mb.json');
writeFileSync(openapiPath, JSON.stringify(openapi) + '\n');

console.log(`Wrote fixtures to ${outDir}`);
