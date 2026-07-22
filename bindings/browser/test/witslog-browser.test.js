'use strict';

const { test } = require('node:test');
const assert = require('node:assert');

const { buildBatch, makeErrorEvent } = require('../witslog-browser');

test('buildBatch wraps events with application', () => {
  const batch = buildBatch([{ message: 'boom' }], { app: 'my-app' });
  assert.strictEqual(batch.application, 'my-app');
  assert.strictEqual(batch.events.length, 1);
  assert.strictEqual(batch.events[0].message, 'boom');
});

test('buildBatch defaults severity to error', () => {
  const batch = buildBatch([{ message: 'boom' }], { app: 'app' });
  assert.strictEqual(batch.events[0].severity, 'error');
});

test('buildBatch keeps caller-supplied severity', () => {
  const batch = buildBatch([{ message: 'boom', severity: 'warn' }], { app: 'app' });
  assert.strictEqual(batch.events[0].severity, 'warn');
});

test('buildBatch forwards error_code/correlation_id/tags', () => {
  const batch = buildBatch(
    [{ message: 'boom', error_code: 'X', correlation_id: 'c1', tags: ['a', 'b'] }],
    { app: 'app' }
  );
  assert.strictEqual(batch.events[0].error_code, 'X');
  assert.strictEqual(batch.events[0].correlation_id, 'c1');
  assert.deepStrictEqual(batch.events[0].tags, ['a', 'b']);
});

test('makeErrorEvent stringifies message', () => {
  const e = makeErrorEvent(42);
  assert.strictEqual(e.message, '42');
  assert.strictEqual(e.severity, 'error');
});

test('makeErrorEvent falls back on null message', () => {
  const e = makeErrorEvent(null);
  assert.strictEqual(e.message, 'error');
});

test('makeErrorEvent carries exception/stacktrace/context through', () => {
  const e = makeErrorEvent('boom', {
    exception: 'TypeError',
    stacktrace: 'at foo.js:1',
    context: { url: 'https://example.com' },
  });
  assert.strictEqual(e.exception, 'TypeError');
  assert.strictEqual(e.stacktrace, 'at foo.js:1');
  assert.deepStrictEqual(e.context, { url: 'https://example.com' });
});

test('init throws without an endpoint', () => {
  const { init } = require('../witslog-browser');
  assert.throws(() => init({}), TypeError);
});
