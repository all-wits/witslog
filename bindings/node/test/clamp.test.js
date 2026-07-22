'use strict';

// Unit tests for lib/clamp.js's clampContext — the shared bounding logic
// that lets the browser-ingest endpoint (frameworks/express.js) and the
// React Query adapter (frameworks/react-query.js) forward a whole `context`
// object instead of just `context.url`, without risking unbounded/DoS-shaped
// input reaching storage (and, downstream, an MCP-connected LLM).

const { test } = require('node:test');
const assert = require('node:assert');

const { clampContext } = require('../lib/clamp');

test('passes through a small, flat object of string/number/boolean leaves', () => {
  const out = clampContext({ a: 'x', b: 1, c: true });
  assert.deepStrictEqual(out, { a: 'x', b: 1, c: true });
});

test('returns undefined for non-object / array / null input', () => {
  assert.strictEqual(clampContext(null), undefined);
  assert.strictEqual(clampContext('str'), undefined);
  assert.strictEqual(clampContext([1, 2]), undefined);
  assert.strictEqual(clampContext(undefined), undefined);
});

test('clamps long string leaves to maxStringLen', () => {
  const out = clampContext({ a: 'x'.repeat(1000) }, { maxStringLen: 10 });
  assert.strictEqual(out.a.length, 10);
});

test('drops functions/symbols, keeps everything else', () => {
  const out = clampContext({ fn: () => {}, sym: Symbol('x'), ok: 'kept' });
  assert.deepStrictEqual(out, { ok: 'kept' });
});

test('caps key count at maxKeys', () => {
  const big = {};
  for (let i = 0; i < 50; i++) big[`k${i}`] = i;
  const out = clampContext(big, { maxKeys: 5 });
  assert.strictEqual(Object.keys(out).length, 5);
});

test('nests up to maxDepth, drops anything deeper', () => {
  const nested = { a: { b: { c: { d: 'too deep' } } } };
  const out = clampContext(nested, { maxDepth: 2 });
  assert.strictEqual(out.a.b.c, undefined); // depth 2 objects allowed, their object children are not
});

test('caps array length at maxArrayLen', () => {
  const out = clampContext({ list: Array.from({ length: 30 }, (_, i) => i) }, { maxArrayLen: 3 });
  assert.strictEqual(out.list.length, 3);
});

test('oversized-after-clamping payload collapses to {_truncated:true} rather than a partial/ambiguous object', () => {
  const huge = {};
  for (let i = 0; i < 20; i++) huge[`k${i}`] = 'x'.repeat(500);
  const out = clampContext(huge, { maxTotalLen: 100 });
  assert.deepStrictEqual(out, { _truncated: true });
});

test('typical React Query mutation-error shape survives intact', () => {
  const ctx = {
    mutationKey: ['cards', 'update'],
    variables: { boardId: 'b1', id: 'c1', input: { title: 'new title' } },
    error: { status: 409, code: 'CARD_VERSION_CONFLICT' },
  };
  const out = clampContext(ctx);
  assert.deepStrictEqual(out.mutationKey, ['cards', 'update']);
  assert.strictEqual(out.variables.boardId, 'b1');
  assert.strictEqual(out.variables.input.title, 'new title');
  assert.strictEqual(out.error.code, 'CARD_VERSION_CONFLICT');
});
