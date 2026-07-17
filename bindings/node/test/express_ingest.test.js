'use strict';

const { test } = require('node:test');
const assert = require('node:assert');

const witslog = require('../index');
const { witslogBrowserIngest, __isLoopback, __clampString, __clampSeverity } = require('../frameworks/express');

// --- pure guardrail helpers --------------------------------------------

test('isLoopback accepts only loopback addresses', () => {
  assert.ok(__isLoopback('127.0.0.1'));
  assert.ok(__isLoopback('::1'));
  assert.ok(__isLoopback('::ffff:127.0.0.1'));
  assert.ok(!__isLoopback('10.0.0.5'));
  assert.ok(!__isLoopback('203.0.113.7'));
});

test('clampString truncates long strings and passes short ones through', () => {
  assert.strictEqual(__clampString('short', 10), 'short');
  assert.strictEqual(__clampString('a'.repeat(20), 5), 'aaaaa');
  assert.strictEqual(__clampString(42, 5), undefined);
  assert.strictEqual(__clampString(undefined, 5), undefined);
});

/** Regression lock: untrusted browser input can never claim fatal/critical. */
test('clampSeverity never allows fatal or critical from untrusted input', () => {
  assert.strictEqual(__clampSeverity('warn'), 'warn');
  assert.strictEqual(__clampSeverity('error'), 'error');
  assert.strictEqual(__clampSeverity('fatal'), 'error');
  assert.strictEqual(__clampSeverity('critical'), 'error');
  assert.strictEqual(__clampSeverity('bogus'), 'error');
  assert.strictEqual(__clampSeverity(undefined), 'error');
});

// --- full handler, mocked req/res --------------------------------------

function fakeReqRes({ origin, remoteAddress = '127.0.0.1', body }) {
  const headers = { origin, 'user-agent': 'test-agent' };
  const req = {
    path: '/__witslog',
    socket: { remoteAddress },
    get: (name) => headers[name.toLowerCase()],
    _listeners: {},
    on(event, cb) {
      this._listeners[event] = cb;
      return this;
    },
    destroy() {
      this._destroyed = true;
    },
    __end(bodyStr) {
      // Simulate the request body streaming in as a single chunk.
      if (this._listeners.data) this._listeners.data(Buffer.from(bodyStr));
      if (this._listeners.end) this._listeners.end();
    },
  };
  const res = {
    statusCode: null,
    body: null,
    status(code) {
      this.statusCode = code;
      return this;
    },
    json(payload) {
      this.body = payload;
      return this;
    },
    end() {
      return this;
    },
  };
  req.__bodyStr = JSON.stringify(body);
  return { req, res };
}

test('rejects requests with a disallowed/missing Origin (fail-closed default [])', () => {
  const middleware = witslogBrowserIngest({ force: true });
  const { req, res } = fakeReqRes({ origin: 'https://evil.example', body: { events: [] } });

  middleware(req, res, () => assert.fail('next() should not be called'));

  assert.strictEqual(res.statusCode, 403);
});

test('rejects non-loopback remote addresses (defense-in-depth)', () => {
  const middleware = witslogBrowserIngest({ force: true, allowedOrigins: ['http://localhost:5173'] });
  const { req, res } = fakeReqRes({
    origin: 'http://localhost:5173',
    remoteAddress: '203.0.113.7',
    body: { events: [] },
  });

  middleware(req, res, () => assert.fail('next() should not be called'));

  assert.strictEqual(res.statusCode, 403);
});

test('refuses to arm when NODE_ENV=production unless forced', () => {
  const prev = process.env.NODE_ENV;
  process.env.NODE_ENV = 'production';
  try {
    assert.throws(() => witslogBrowserIngest({}), /NODE_ENV=production/);
    assert.doesNotThrow(() => witslogBrowserIngest({ force: true }));
  } finally {
    if (prev === undefined) delete process.env.NODE_ENV;
    else process.env.NODE_ENV = prev;
  }
});

test('accepts an allowed origin and persists clamped events via the SDK', () => {
  const logged = [];
  witslog.__setLibForTest({ log: (json) => ((logged.push(JSON.parse(json))), 1) });

  const middleware = witslogBrowserIngest({
    force: true,
    allowedOrigins: ['http://localhost:5173'],
    application: 'my-web-app',
  });
  const { req, res } = fakeReqRes({
    origin: 'http://localhost:5173',
    body: {
      events: [{ message: 'boom', severity: 'fatal', context: { url: 'http://localhost:5173/page' } }],
    },
  });

  middleware(req, res, () => assert.fail('next() should not be called'));
  req.__end(req.__bodyStr);

  assert.strictEqual(res.statusCode, 202);
  assert.strictEqual(logged.length, 1);
  assert.strictEqual(logged[0].application, 'my-web-app');
  assert.strictEqual(logged[0].message, 'boom');
  // fatal was clamped to error — untrusted input can't claim fatal.
  assert.strictEqual(logged[0].severity, 'error');
  assert.deepStrictEqual(logged[0].tags, ['browser']);
  assert.strictEqual(logged[0].context.url, 'http://localhost:5173/page');
});

test('rate limit rejects requests past the configured max', () => {
  witslog.__setLibForTest({ log: () => 1 });

  const middleware = witslogBrowserIngest({
    force: true,
    allowedOrigins: ['http://localhost:5173'],
    rateLimit: { windowMs: 60_000, max: 1 },
  });

  const first = fakeReqRes({ origin: 'http://localhost:5173', body: { events: [] } });
  middleware(first.req, first.res, () => assert.fail('next() should not be called'));
  first.req.__end(first.req.__bodyStr);
  assert.strictEqual(first.res.statusCode, 202);

  const second = fakeReqRes({ origin: 'http://localhost:5173', body: { events: [] } });
  middleware(second.req, second.res, () => assert.fail('next() should not be called'));
  assert.strictEqual(second.res.statusCode, 429);
});

test('oversized body is rejected with 413', () => {
  witslog.__setLibForTest({ log: () => 1 });

  const middleware = witslogBrowserIngest({
    force: true,
    allowedOrigins: ['http://localhost:5173'],
    maxBytes: 10,
  });
  const { req, res } = fakeReqRes({
    origin: 'http://localhost:5173',
    body: { events: [{ message: 'this body is way over ten bytes' }] },
  });

  middleware(req, res, () => assert.fail('next() should not be called'));
  req.__end(req.__bodyStr);

  assert.strictEqual(res.statusCode, 413);
});

test('a malformed event in the batch does not break the rest of the batch', () => {
  const logged = [];
  witslog.__setLibForTest({
    log: (json) => {
      const parsed = JSON.parse(json);
      if (parsed.message === 'bad') throw new Error('simulated failure');
      logged.push(parsed);
      return 1;
    },
  });

  const middleware = witslogBrowserIngest({
    force: true,
    allowedOrigins: ['http://localhost:5173'],
  });
  const { req, res } = fakeReqRes({
    origin: 'http://localhost:5173',
    body: { events: [{ message: 'bad' }, { message: 'good' }] },
  });

  middleware(req, res, () => assert.fail('next() should not be called'));
  req.__end(req.__bodyStr);

  assert.strictEqual(res.statusCode, 202);
  assert.strictEqual(logged.length, 1);
  assert.strictEqual(logged[0].message, 'good');
});
