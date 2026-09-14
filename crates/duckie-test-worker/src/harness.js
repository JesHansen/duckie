// Captured intrinsics and closures keep test output bounded even if scripts alter globals.
(() => {
  const input = JSON.parse(__duckieInput);
  delete globalThis.__duckieInput;
  const stringify = JSON.stringify.bind(JSON);
  const parse = JSON.parse.bind(JSON);
  const tests = [], logs = [];
  let logSize = 0, suiteError = null, parsed, hasParsed = false;
  const bounded = value => {
    try { return String(value).slice(0, 4096); } catch { return '[unprintable]'; }
  };
  const display = value => {
    try { return bounded(stringify(value)); } catch { return bounded(value); }
  };
  const errorText = e => bounded(e && (e.message && e.stack ? `${e.message}\n${e.stack}` : e.message || e.stack) || e);
  const failureLine = text => {
    const m = /at (?:.*\()?tests\.js:(\d+):\d+/.exec(text);
    return m ? Number(m[1]) : null;
  };
  const freeze = value => {
    if (value && typeof value === 'object') {
      Object.values(value).forEach(freeze); Object.freeze(value);
    }
    return value;
  };
  const body = () => {
    if (input.body === null) throw new Error('Body unavailable at this limit: complete body exceeds 10 MiB');
    return input.body;
  };
  const json = () => { if (!hasParsed) { parsed = parse(body()); hasParsed = true; } return parsed; };
  const jsonPointer = pointer => {
    if (pointer === '') return json();
    if (typeof pointer !== 'string' || !pointer.startsWith('/')) throw new Error('JSON Pointer must be empty or start with /');
    return pointer.slice(1).split('/').reduce((value, part) => {
      const key = part.replace(/~1/g, '/').replace(/~0/g, '~');
      if (value === null || typeof value !== 'object' || !Object.hasOwn(value, key)) throw new Error(`JSON Pointer not found: ${pointer}`);
      return value[key];
    }, json());
  };
  Object.defineProperty(globalThis, 'response', { value: freeze({
    status: input.status, bodySize: input.bodySize, durationMs: input.durationMs,
    header: name => { const row = input.headers.find(h => h[0].toLowerCase() === String(name).toLowerCase()); return row ? row[1] : null; },
    headers: name => input.headers.filter(h => h[0].toLowerCase() === String(name).toLowerCase()).map(h => h[1]),
    text: body,
    json, jsonPointer
  }) });
  Object.defineProperty(globalThis, 'environment', { value: freeze(input.environment) });
  Object.defineProperty(globalThis, 'request', { value: freeze(input.request) });
  const deepEqual = (a, b) => {
    if (a === b) return true;
    if (!a || !b || typeof a !== 'object' || typeof b !== 'object' || Array.isArray(a) !== Array.isArray(b)) return false;
    const ka = Object.keys(a), kb = Object.keys(b);
    return ka.length === kb.length && ka.every(k => Object.hasOwn(b, k) && deepEqual(a[k], b[k]));
  };
  Object.defineProperty(globalThis, 'expect', { value: (actual, path) => {
    const check = (ok, expected, operator) => { if (!ok) throw new Error(`Assertion failed (${operator})${path ? `\nPath: ${bounded(path)}` : ''}\nExpected: ${display(expected)}\nReceived: ${display(actual)}`); };
    const numeric = (expected, predicate, operator) => check(typeof actual === 'number' && typeof expected === 'number' && predicate(actual, expected), expected, operator);
    return {
      toBe: expected => check((actual === null || typeof actual !== 'object') && actual === expected, expected, 'toBe'),
      toEqual: expected => check(deepEqual(actual, expected), expected, 'toEqual'),
      toContain: expected => check((typeof actual === 'string' || Array.isArray(actual)) && actual.includes(expected), expected, 'toContain'),
      toBeType: expected => check((actual === null ? 'null' : Array.isArray(actual) ? 'array' : typeof actual) === expected, expected, 'toBeType'),
      toBeLessThan: expected => numeric(expected, (a,b) => a < b, 'toBeLessThan'),
      toBeLessThanOrEqual: expected => numeric(expected, (a,b) => a <= b, 'toBeLessThanOrEqual'),
      toBeGreaterThan: expected => numeric(expected, (a,b) => a > b, 'toBeGreaterThan'),
      toBeGreaterThanOrEqual: expected => numeric(expected, (a,b) => a >= b, 'toBeGreaterThanOrEqual')
    };
  } });
  Object.defineProperty(globalThis, 'test', { value: (name, fn) => {
    if (tests.length >= 200) throw new Error('Test count limit reached (200)');
    try {
      const returned = fn();
      if (returned && typeof returned.then === 'function') throw new Error('Unsupported async test: use a synchronous callback');
      tests.push({ name: bounded(name).slice(0,256), passed: true, error: null, line: null });
    } catch (e) {
      const text = errorText(e);
      tests.push({ name: bounded(name).slice(0,256), passed: false, error: text.slice(0,768), line: failureLine(text) });
    }
  } });
  const log = (...args) => {
    if (logs.length >= 1000 || logSize >= 32768) return;
    const text = args.map(display).join(' ').slice(0, Math.min(2048, 32768 - logSize));
    logs.push(text); logSize += text.length;
  };
  Object.defineProperty(globalThis, 'console', { value: freeze({ log, warn: log, error: log, info: log }) });
  return { fail: e => { suiteError = errorText(e); }, report: () => stringify({ tests, console: logs, suiteError, durationMs: 0 }) };
})();
