'use strict';

// Unit tests for the Next.js adapter (bindings/node/frameworks/next.js) —
// proves onRequestError/withWitslog capture request context + cause chains
// with zero code in the failing route, mirroring the express/flask adapter
// tests' fake-lib style.

const { test, beforeEach } = require('node:test');
const assert = require('node:assert');

const witslog = require('../index');
const next = require('../frameworks/next');

let captured;

beforeEach(() => {
  captured = null;
  next.__resetForTest();
  witslog.__setLibForTest({
    init: () => 0,
    log: (json) => {
      captured = JSON.parse(json);
      return 1;
    },
  });
});

test('register() mounts witslog once; a second call is a no-op', () => {
  next.register('witsnote-proxy');
  next.register('should-be-ignored');
  const err = new Error('boom');
  next.onRequestError(err, { method: 'GET', path: '/x' }, {});
  assert.strictEqual(captured.application, 'witsnote-proxy');
});

test('onRequestError captures method/path and Next router context', () => {
  next.register('witsnote-proxy');
  const err = new Error('boom');

  next.onRequestError(
    err,
    { method: 'PUT', path: '/api/proxy/cards/abc' },
    { routerKind: 'App Router', routePath: '/api/proxy/[...path]', routeType: 'route' }
  );

  assert.ok(captured);
  assert.strictEqual(captured.severity, 'error');
  assert.strictEqual(captured.context.http.method, 'PUT');
  assert.strictEqual(captured.context.http.path, '/api/proxy/cards/abc');
  assert.strictEqual(captured.context.next.routePath, '/api/proxy/[...path]');
  assert.ok(captured.tags.includes('next'));
  assert.ok(captured.tags.includes('route'));
});

test('onRequestError falls back to application "next" when register() was never called', () => {
  next.onRequestError(new Error('boom'), { method: 'GET', path: '/x' }, {});
  assert.strictEqual(captured.application, 'next');
});

test('onRequestError unwraps a fetch-style cause chain (via index.js exception())', () => {
  next.register('witsnote-proxy');
  const cause = new Error('connect ECONNREFUSED 127.0.0.1:8000');
  cause.code = 'ECONNREFUSED';
  const err = new TypeError('fetch failed', { cause });

  next.onRequestError(err, { method: 'POST', path: '/api/proxy/cards' }, { routeType: 'route' });

  assert.strictEqual(captured.context.root_cause, 'ECONNREFUSED');
  assert.match(captured.stacktrace, /Caused by: ECONNREFUSED/);
});

test('onRequestError never throws even if the underlying log call fails', () => {
  next.register('app');
  witslog.__setLibForTest({
    log: () => {
      throw new Error('DB locked');
    },
  });
  assert.doesNotThrow(() => next.onRequestError(new Error('boom'), { method: 'GET', path: '/x' }, {}));
});

test('withWitslog: handler succeeds -> return value passes through, nothing logged', async () => {
  const handler = async () => new Response('ok');
  const wrapped = next.withWitslog(handler, { application: 'app' });
  const res = await wrapped({ method: 'GET', nextUrl: { pathname: '/x' } });
  assert.strictEqual(await res.text(), 'ok');
  assert.strictEqual(captured, null);
});

test('withWitslog: handler throws -> logs with method/path/latency, then rethrows the original error', async () => {
  const boom = new Error('handler exploded');
  const handler = async () => {
    throw boom;
  };
  const wrapped = next.withWitslog(handler, { application: 'witsnote-proxy', tags: ['custom'] });

  await assert.rejects(
    () => wrapped({ method: 'DELETE', nextUrl: { pathname: '/api/proxy/cards/1' } }),
    (err) => err === boom
  );

  assert.ok(captured);
  assert.strictEqual(captured.context.http.method, 'DELETE');
  assert.strictEqual(captured.context.http.path, '/api/proxy/cards/1');
  assert.ok(typeof captured.context.timing.latency_ms === 'number');
  assert.ok(captured.tags.includes('custom'));
  assert.ok(captured.tags.includes('route-handler'));
});
