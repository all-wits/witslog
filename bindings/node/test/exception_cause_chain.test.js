'use strict';

// Regression lock for the exception() cause-chain fix: Node's own `fetch`
// (undici) throws `TypeError: fetch failed` whose real reason (ECONNREFUSED/
// ETIMEDOUT/ENOTFOUND/etc) lives on `.cause`, not on the top-level Error. Prior
// to this fix, exception() dropped `.cause` entirely — every wrapped fetch
// failure logged as an undiagnosable bare "fetch failed". See CLAUDE.md
// gotcha "SDK exception() drops the cause chain".

const { test } = require('node:test');
const assert = require('node:assert');

const witslog = require('../index');

test('unwrapCauseChain walks a single-level .cause and extracts code as root cause', () => {
  const cause = new Error('connect ECONNREFUSED 127.0.0.1:8000');
  cause.code = 'ECONNREFUSED';
  const err = new Error('fetch failed', { cause });

  const { lines, rootCause } = witslog.__unwrapCauseChain(err);

  assert.strictEqual(rootCause, 'ECONNREFUSED');
  assert.strictEqual(lines.length, 1);
  assert.match(lines[0], /^Caused by: ECONNREFUSED Error: connect ECONNREFUSED/);
});

test('unwrapCauseChain walks multi-level cause chains, deepest code wins', () => {
  const root = new Error('socket hang up');
  root.code = 'ECONNRESET';
  const mid = new Error('request failed', { cause: root });
  const err = new Error('fetch failed', { cause: mid });

  const { lines, rootCause } = witslog.__unwrapCauseChain(err);

  assert.strictEqual(rootCause, 'ECONNRESET');
  assert.strictEqual(lines.length, 2);
});

test('unwrapCauseChain returns no lines / undefined rootCause when there is no cause', () => {
  const err = new Error('plain error');
  const { lines, rootCause } = witslog.__unwrapCauseChain(err);
  assert.strictEqual(lines.length, 0);
  assert.strictEqual(rootCause, undefined);
});

test('unwrapCauseChain caps depth at 10 against a pathological/cyclic cause chain', () => {
  let err = new Error('base');
  err.code = 'BASE';
  for (let i = 0; i < 20; i++) {
    const next = new Error(`level ${i}`);
    next.cause = err;
    err = next;
  }
  const { lines } = witslog.__unwrapCauseChain(err);
  assert.ok(lines.length <= 10);
});

test('exception() folds cause chain into stacktrace and context.root_cause, never a top-level field', () => {
  let captured = null;
  witslog.__setLibForTest({
    log: (json) => {
      captured = JSON.parse(json);
      return 1;
    },
  });

  const cause = new Error('connect ECONNREFUSED 127.0.0.1:8000');
  cause.code = 'ECONNREFUSED';
  const err = new TypeError('fetch failed', { cause });

  witslog.exception('witsnote-proxy', err, { context: { path: '/cards' } });

  assert.ok(captured, 'log() should have been called');
  // root_cause is a Rust-only EventBuilder field, NOT part of the witslog_log
  // JSON contract (bindings/CONTRACT.md) — must never appear as a top-level key.
  assert.strictEqual(captured.root_cause, undefined);
  assert.strictEqual(captured.context.root_cause, 'ECONNREFUSED');
  assert.strictEqual(captured.context.path, '/cards'); // caller's context preserved
  assert.match(captured.stacktrace, /Caused by: ECONNREFUSED Error: connect ECONNREFUSED/);
});

test('exception() does not overwrite an explicitly-provided context.root_cause', () => {
  let captured = null;
  witslog.__setLibForTest({
    log: (json) => {
      captured = JSON.parse(json);
      return 1;
    },
  });

  const cause = new Error('boom');
  cause.code = 'ESOMETHING';
  const err = new Error('wrapped', { cause });

  witslog.exception('app', err, { context: { root_cause: 'EXPLICIT' } });

  assert.strictEqual(captured.context.root_cause, 'EXPLICIT');
});

test('exception() with no .cause behaves exactly as before (no context mutation)', () => {
  let captured = null;
  witslog.__setLibForTest({
    log: (json) => {
      captured = JSON.parse(json);
      return 1;
    },
  });

  witslog.exception('app', new Error('plain'));

  assert.ok(captured);
  assert.strictEqual(captured.context, undefined);
  assert.strictEqual(captured.exception, 'Error');
});
