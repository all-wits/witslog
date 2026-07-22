'use strict';

// Unit tests for witslogFetch (bindings/node/fetch.js) — the instrumented
// fetch wrapper that replaces per-call-site try/catch + witslog.exception/
// error boilerplate (e.g. in WitsNote's proxy route.ts). Global `fetch` is
// stubbed so these run with no real network I/O.

const { test, beforeEach, afterEach } = require('node:test');
const assert = require('node:assert');

const witslog = require('../index');
const { witslogFetch } = require('../fetch');

let originalFetch;
let captured;

function fakeLib() {
  return {
    log: (json) => {
      captured = JSON.parse(json);
      return 1;
    },
  };
}

beforeEach(() => {
  originalFetch = global.fetch;
  captured = null;
  witslog.__setLibForTest(fakeLib());
});

afterEach(() => {
  global.fetch = originalFetch;
});

test('successful 2xx response: passes through untouched, logs nothing', async () => {
  global.fetch = async () => new Response('{"ok":true}', { status: 200 });
  const res = await witslogFetch('http://localhost:8000/api/cards', undefined, {
    application: 'witsnote-proxy',
  });
  assert.strictEqual(res.status, 200);
  assert.strictEqual(captured, null);
});

test('409 response: logs at warn with error_code/body from the upstream contract shape', async () => {
  const body = JSON.stringify({
    error: { code: 'CARD_VERSION_CONFLICT', message: 'card modified by another client', details: { expected_version: 7 } },
  });
  global.fetch = async () => new Response(body, { status: 409 });

  const res = await witslogFetch('http://localhost:8000/api/cards/abc', { method: 'PUT' }, {
    application: 'witsnote-proxy',
  });

  assert.strictEqual(res.status, 409); // caller still gets the real response
  assert.ok(captured, 'a witslog event should have been logged');
  assert.strictEqual(captured.severity, 'warn'); // expected 4xx -> warn (decision)
  assert.strictEqual(captured.error_code, 'CARD_VERSION_CONFLICT');
  assert.ok(captured.tags.includes('upstream-4xx'));
  assert.ok(!captured.tags.includes('retryable'));
  assert.strictEqual(captured.context.http.status, 409);
  assert.strictEqual(captured.context.upstream.error_code, 'CARD_VERSION_CONFLICT');
  assert.deepStrictEqual(captured.context.upstream.details, { expected_version: 7 });
  assert.ok(typeof captured.context.timing.latency_ms === 'number');
  assert.ok(captured.correlation_id);
});

test('response body is still readable by the caller after a non-2xx capture (clone, not consume)', async () => {
  const body = JSON.stringify({ error: { code: 'X', message: 'boom' } });
  global.fetch = async () => new Response(body, { status: 400 });

  const res = await witslogFetch('http://localhost:8000/api/x', undefined, { application: 'app' });
  const parsed = await res.json();
  assert.strictEqual(parsed.error.code, 'X');
});

test('500 response: logs at error, tagged upstream-5xx + retryable', async () => {
  global.fetch = async () => new Response('Internal Server Error', { status: 500 });
  await witslogFetch('http://localhost:8000/api/cards', undefined, { application: 'witsnote-proxy' });

  assert.strictEqual(captured.severity, 'error'); // 5xx -> error (decision)
  assert.strictEqual(captured.error_code, 'HTTP_500');
  assert.ok(captured.tags.includes('upstream-5xx'));
  assert.ok(captured.tags.includes('retryable'));
});

test('thrown fetch error (network unreachable): logs exception with cause + UPSTREAM_UNREACHABLE, then rethrows', async () => {
  const cause = new Error('connect ECONNREFUSED 127.0.0.1:8000');
  cause.code = 'ECONNREFUSED';
  global.fetch = async () => {
    throw new TypeError('fetch failed', { cause });
  };

  await assert.rejects(
    () => witslogFetch('http://localhost:8000/api/cards', { method: 'POST' }, { application: 'witsnote-proxy' }),
    (err) => err instanceof TypeError && err.message === 'fetch failed'
  );

  assert.ok(captured, 'a witslog event should have been logged before rethrow');
  assert.strictEqual(captured.severity, 'error');
  assert.strictEqual(captured.error_code, 'UPSTREAM_UNREACHABLE');
  assert.strictEqual(captured.context.root_cause, 'ECONNREFUSED');
  assert.match(captured.stacktrace, /Caused by: ECONNREFUSED/);
  assert.ok(captured.tags.includes('upstream-unreachable'));
  assert.strictEqual(captured.context.http.method, 'POST');
});

test('correlation id: mints one when absent, and it is set on the outbound request header', async () => {
  let seenHeader;
  global.fetch = async (input, init) => {
    seenHeader = new Headers(init && init.headers).get('x-request-id');
    return new Response('{}', { status: 200 });
  };
  await witslogFetch('http://localhost:8000/api/cards', {}, { application: 'app' });
  assert.ok(seenHeader, 'x-request-id should have been set on the outbound request');
});

test('correlation id: reuses opts.correlationId when supplied (e.g. from an inbound request)', async () => {
  let seenHeader;
  global.fetch = async (input, init) => {
    seenHeader = new Headers(init && init.headers).get('x-request-id');
    return new Response(JSON.stringify({ error: { code: 'E' } }), { status: 400 });
  };
  await witslogFetch(
    'http://localhost:8000/api/cards',
    {},
    { application: 'app', correlationId: 'inbound-abc-123' }
  );
  assert.strictEqual(seenHeader, 'inbound-abc-123');
  assert.strictEqual(captured.correlation_id, 'inbound-abc-123');
});

test('non-JSON error body: still logs with a clamped raw-body snapshot and generic HTTP_<status> code', async () => {
  global.fetch = async () => new Response('<html>502 Bad Gateway</html>', { status: 502 });
  await witslogFetch('http://localhost:8000/api/cards', undefined, { application: 'app' });
  assert.strictEqual(captured.error_code, 'HTTP_502');
  assert.ok(captured.context.upstream.body.includes('Bad Gateway'));
});

test('a logging failure never masks the real response for the caller', async () => {
  global.fetch = async () => new Response('{"error":{"code":"X"}}', { status: 400 });
  witslog.__setLibForTest({
    log: () => {
      throw new Error('DB is locked');
    },
  });
  const res = await witslogFetch('http://localhost:8000/api/cards', undefined, { application: 'app' });
  assert.strictEqual(res.status, 400); // did not throw, caller still gets the response
});
