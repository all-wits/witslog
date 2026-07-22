'use strict';

// Unit tests for witslogNextIngest (frameworks/next.js) — the Next.js
// Route Handler-shaped counterpart to witslogBrowserIngest (Express).
// A real Express req/res cannot be reused for a Next.js Route Handler
// (Web Request/Response API vs Node's raw req/res) — this is a genuine
// second entry point sharing lib/ingest-core.js's guardrail/persist logic,
// not a re-export, so it needs its own coverage.

const { test, beforeEach } = require('node:test');
const assert = require('node:assert');

const witslog = require('../index');
const { witslogNextIngest } = require('../frameworks/next');

function req(body, { origin = 'http://localhost:5173', headers = {} } = {}) {
  return new Request('http://localhost:3000/api/__witslog', {
    method: 'POST',
    headers: { origin, 'content-type': 'application/json', ...headers },
    body: JSON.stringify(body),
  });
}

beforeEach(() => {
  witslog.__setLibForTest({ log: () => 1 });
});

test('refuses to arm when NODE_ENV=production unless forced', () => {
  const prev = process.env.NODE_ENV;
  process.env.NODE_ENV = 'production';
  try {
    assert.throws(() => witslogNextIngest({}), /NODE_ENV=production/);
    assert.doesNotThrow(() => witslogNextIngest({ force: true }));
  } finally {
    if (prev === undefined) delete process.env.NODE_ENV;
    else process.env.NODE_ENV = prev;
  }
});

test('rejects a disallowed/missing Origin (fail-closed default [])', async () => {
  const handler = witslogNextIngest({ force: true });
  const res = await handler(req({ events: [] }, { origin: 'https://evil.example' }));
  assert.strictEqual(res.status, 403);
});

test('accepts an allowed origin and persists clamped events via the SDK', async () => {
  const logged = [];
  witslog.__setLibForTest({ log: (json) => ((logged.push(JSON.parse(json))), 1) });

  const handler = witslogNextIngest({
    force: true,
    allowedOrigins: ['http://localhost:5173'],
    application: 'witsnote-client',
  });

  const res = await handler(
    req({
      events: [
        {
          message: 'mutation cards.update failed',
          severity: 'error',
          error_code: 'CARD_VERSION_CONFLICT',
          tags: ['react-query', 'mutation'],
          context: { mutationKey: ['cards', 'update'], variables: { id: 'c1' } },
        },
      ],
    })
  );

  assert.strictEqual(res.status, 202);
  assert.strictEqual(logged.length, 1);
  assert.strictEqual(logged[0].application, 'witsnote-client');
  assert.strictEqual(logged[0].error_code, 'CARD_VERSION_CONFLICT');
  assert.deepStrictEqual(logged[0].tags, ['browser', 'react-query', 'mutation']);
  assert.deepStrictEqual(logged[0].context.mutationKey, ['cards', 'update']);
});

test('rate limit rejects requests past the configured max', async () => {
  const handler = witslogNextIngest({
    force: true,
    allowedOrigins: ['http://localhost:5173'],
    rateLimit: { windowMs: 60_000, max: 1 },
  });

  const first = await handler(req({ events: [] }));
  assert.strictEqual(first.status, 202);

  const second = await handler(req({ events: [] }));
  assert.strictEqual(second.status, 429);
});

test('oversized body (post-buffer check) is rejected with 413', async () => {
  const handler = witslogNextIngest({
    force: true,
    allowedOrigins: ['http://localhost:5173'],
    maxBytes: 10,
  });
  const res = await handler(req({ events: [{ message: 'this body is way over ten bytes' }] }));
  assert.strictEqual(res.status, 413);
});

test('invalid JSON body returns 400', async () => {
  const handler = witslogNextIngest({ force: true, allowedOrigins: ['http://localhost:5173'] });
  const badReq = new Request('http://localhost:3000/api/__witslog', {
    method: 'POST',
    headers: { origin: 'http://localhost:5173' },
    body: 'not json',
  });
  const res = await handler(badReq);
  assert.strictEqual(res.status, 400);
});

test('a malformed event in the batch does not break the rest of the batch', async () => {
  const logged = [];
  witslog.__setLibForTest({
    log: (json) => {
      const parsed = JSON.parse(json);
      if (parsed.message === 'bad') throw new Error('simulated failure');
      logged.push(parsed);
      return 1;
    },
  });

  const handler = witslogNextIngest({ force: true, allowedOrigins: ['http://localhost:5173'] });
  const res = await handler(req({ events: [{ message: 'bad' }, { message: 'good' }] }));

  assert.strictEqual(res.status, 202);
  assert.strictEqual(logged.length, 1);
  assert.strictEqual(logged[0].message, 'good');
});
