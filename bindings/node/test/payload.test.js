'use strict';

const { test } = require('node:test');
const assert = require('node:assert');

const { buildPayload, encode } = require('../lib/payload');

test('payload has required fields', () => {
  assert.deepStrictEqual(buildPayload('app', 'msg'), { application: 'app', message: 'msg' });
});

test('payload passes context/tags/metadata through', () => {
  const p = buildPayload('app', 'msg', {
    context: { request_id: 'r1' },
    tags: ['a', 'b'],
    metadata: { k: 'v' },
    severity: 'warn',
  });
  assert.deepStrictEqual(p.context, { request_id: 'r1' });
  assert.deepStrictEqual(p.tags, ['a', 'b']);
  assert.deepStrictEqual(p.metadata, { k: 'v' });
  assert.strictEqual(p.severity, 'warn');
});

test('payload drops null/undefined fields', () => {
  const p = buildPayload('app', 'msg', { category: null, tags: undefined });
  assert.ok(!('category' in p));
  assert.ok(!('tags' in p));
});

test('payload rejects unknown field', () => {
  assert.throws(() => buildPayload('app', 'msg', { bogus: 1 }), RangeError);
});

test('payload rejects non-string application/message', () => {
  assert.throws(() => buildPayload(1, 'msg'), TypeError);
  assert.throws(() => buildPayload('app', 2), TypeError);
});

test('payload rejects non-array tags', () => {
  assert.throws(() => buildPayload('app', 'msg', { tags: 'x' }), TypeError);
});

test('encode produces JSON string', () => {
  assert.strictEqual(encode({ application: 'a', message: 'm' }), '{"application":"a","message":"m"}');
});
