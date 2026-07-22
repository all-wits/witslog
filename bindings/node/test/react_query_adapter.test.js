'use strict';

// Unit tests for the React Query adapter (bindings/node/frameworks/react-query.js).
// A fake QueryClient (duck-typed: getMutationCache()/getQueryCache() each
// exposing subscribe(listener)) stands in for @tanstack/react-query so these
// run with no real dependency — matches the adapter's own no-hard-dep design.

const { test } = require('node:test');
const assert = require('node:assert');

const { attachWitslog } = require('../frameworks/react-query');

function fakeCache() {
  let listener = null;
  return {
    subscribe(fn) {
      listener = fn;
      return () => {
        listener = null;
      };
    },
    fire(event) {
      if (listener) listener(event);
    },
    hasListener() {
      return listener !== null;
    },
  };
}

function fakeQueryClient() {
  const mutationCache = fakeCache();
  const queryCache = fakeCache();
  return {
    getMutationCache: () => mutationCache,
    getQueryCache: () => queryCache,
    __mutationCache: mutationCache,
    __queryCache: queryCache,
  };
}

test('attachWitslog requires a report sink (function or {enqueue})', () => {
  const qc = fakeQueryClient();
  assert.throws(() => attachWitslog(qc, {}), TypeError);
  assert.doesNotThrow(() => attachWitslog(qc, { report: () => {} }));
  assert.doesNotThrow(() => attachWitslog(qc, { report: { enqueue: () => {} } } ));
});

test('captures a failed mutation: mutationKey, variables, and the error', () => {
  const qc = fakeQueryClient();
  const events = [];
  attachWitslog(qc, { report: (e) => events.push(e), tags: ['witsnote'] });

  const err = Object.assign(new Error('card modified by another client'), {
    name: 'ApiError',
    code: 'CARD_VERSION_CONFLICT',
    status: 409,
  });

  qc.__mutationCache.fire({
    type: 'updated',
    mutation: {
      options: { mutationKey: ['cards', 'update'] },
      state: { status: 'error', error: err, variables: { id: 'c1', input: { title: 'new' } } },
    },
  });

  assert.strictEqual(events.length, 1);
  const evt = events[0];
  assert.strictEqual(evt.message, 'card modified by another client');
  assert.strictEqual(evt.error_code, 'CARD_VERSION_CONFLICT');
  assert.strictEqual(evt.context.http_status, 409);
  assert.deepStrictEqual(evt.context.mutationKey, ['cards', 'update']);
  assert.deepStrictEqual(evt.context.variables, { id: 'c1', input: { title: 'new' } });
  assert.deepStrictEqual(evt.tags, ['witsnote', 'react-query', 'mutation']);
});

test('ignores mutation cache events that are not a failed/updated mutation', () => {
  const qc = fakeQueryClient();
  const events = [];
  attachWitslog(qc, { report: (e) => events.push(e) });

  qc.__mutationCache.fire({ type: 'added', mutation: { state: { status: 'pending' } } });
  qc.__mutationCache.fire({
    type: 'updated',
    mutation: { state: { status: 'success', variables: {} } },
  });

  assert.strictEqual(events.length, 0);
});

test('captures a failed query: queryKey and the error', () => {
  const qc = fakeQueryClient();
  const events = [];
  attachWitslog(qc, { report: (e) => events.push(e) });

  qc.__queryCache.fire({
    type: 'updated',
    query: { queryKey: ['cards', 'search', 'foo'], state: { status: 'error', error: new Error('boom') } },
  });

  assert.strictEqual(events.length, 1);
  assert.deepStrictEqual(events[0].context.queryKey, ['cards', 'search', 'foo']);
  assert.deepStrictEqual(events[0].tags, ['react-query', 'query']);
});

test('captureQueries:false disables query-cache subscription entirely', () => {
  const qc = fakeQueryClient();
  const events = [];
  attachWitslog(qc, { report: (e) => events.push(e), captureQueries: false });

  assert.strictEqual(qc.__queryCache.hasListener(), false);
  assert.strictEqual(qc.__mutationCache.hasListener(), true);
});

test('detach() unsubscribes both caches', () => {
  const qc = fakeQueryClient();
  const detach = attachWitslog(qc, { report: () => {} });
  assert.ok(qc.__mutationCache.hasListener());
  assert.ok(qc.__queryCache.hasListener());
  detach();
  assert.strictEqual(qc.__mutationCache.hasListener(), false);
  assert.strictEqual(qc.__queryCache.hasListener(), false);
});

test('report as a {enqueue} object (e.g. WitslogBrowser.init() return value) works too', () => {
  const enqueued = [];
  const qc = fakeQueryClient();
  attachWitslog(qc, { report: { enqueue: (e) => enqueued.push(e) } });

  qc.__mutationCache.fire({
    type: 'updated',
    mutation: { options: {}, state: { status: 'error', error: new Error('x'), variables: {} } },
  });

  assert.strictEqual(enqueued.length, 1);
});

test('computes latency_ms from mutation state.submittedAt/errorUpdatedAt', () => {
  const qc = fakeQueryClient();
  const events = [];
  attachWitslog(qc, { report: (e) => events.push(e) });

  qc.__mutationCache.fire({
    type: 'updated',
    mutation: {
      options: {},
      state: {
        status: 'error',
        error: new Error('boom'),
        variables: {},
        submittedAt: 1000,
        errorUpdatedAt: 1042,
      },
    },
  });

  assert.strictEqual(events[0].context.timing.latency_ms, 42);
});

test('reads correlation_id/latencyMs stamped by witslogAxiosInterceptor onto the error object', () => {
  const qc = fakeQueryClient();
  const events = [];
  attachWitslog(qc, { report: (e) => events.push(e) });

  const err = Object.assign(new Error('unreachable'), {
    correlationId: 'corr-123',
    latencyMs: 77,
  });

  qc.__mutationCache.fire({
    type: 'updated',
    mutation: { options: {}, state: { status: 'error', error: err, variables: {} } },
  });

  assert.strictEqual(events[0].correlation_id, 'corr-123');
  assert.strictEqual(events[0].context.timing.latency_ms, 77);
});

test('a non-Error thrown value (e.g. a rejected string) is normalized to a message', () => {
  const qc = fakeQueryClient();
  const events = [];
  attachWitslog(qc, { report: (e) => events.push(e) });

  qc.__mutationCache.fire({
    type: 'updated',
    mutation: { options: {}, state: { status: 'error', error: 'plain string rejection', variables: {} } },
  });

  assert.strictEqual(events[0].message, 'plain string rejection');
});
