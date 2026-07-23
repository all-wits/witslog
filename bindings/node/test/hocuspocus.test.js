'use strict';

// Unit tests for the Hocuspocus adapter (bindings/node/frameworks/hocuspocus.js).
// A fake EventEmitter-shaped provider (duck-typed: on/off) stands in for a
// real HocuspocusProvider — matches the adapter's no-hard-dep design.

const { test } = require('node:test');
const assert = require('node:assert');

const { attachWitslogHocuspocus, __isAbnormalClose } = require('../frameworks/hocuspocus');

function fakeProvider() {
  const handlers = {};
  return {
    on: (event, fn) => {
      (handlers[event] = handlers[event] || []).push(fn);
    },
    off: (event, fn) => {
      handlers[event] = (handlers[event] || []).filter((h) => h !== fn);
    },
    __emit: (event, payload) => {
      (handlers[event] || []).forEach((fn) => fn(payload));
    },
    __handlerCount: (event) => (handlers[event] || []).length,
  };
}

function fakeReporter() {
  const events = [];
  let flushCount = 0;
  return {
    events,
    enqueue: (e) => events.push(e),
    flush: () => {
      flushCount += 1;
    },
    get flushCount() {
      return flushCount;
    },
  };
}

test('isAbnormalClose: true for abnormal codes, false for 1000/1001 or wasClean', () => {
  assert.strictEqual(__isAbnormalClose(1006, false), true);
  assert.strictEqual(__isAbnormalClose(1000, false), false);
  assert.strictEqual(__isAbnormalClose(1001, false), false);
  assert.strictEqual(__isAbnormalClose(1005, true), false);
});

test('captures + immediately flushes on abnormal close', () => {
  const provider = fakeProvider();
  const reporter = fakeReporter();
  attachWitslogHocuspocus(provider, { report: reporter, tags: ['witsnote'], context: { board: { boardId: 'b1' } } });

  provider.__emit('close', { event: { code: 1006, reason: '', wasClean: false } });

  assert.strictEqual(reporter.events.length, 1);
  assert.strictEqual(reporter.events[0].error_code, 'WS_CLOSE_1006');
  assert.deepStrictEqual(reporter.events[0].tags, ['network', 'websocket', 'witsnote']);
  assert.strictEqual(reporter.events[0].context.board.boardId, 'b1');
  assert.strictEqual(reporter.flushCount, 1);
});

test('does not capture a clean close (code 1000)', () => {
  const provider = fakeProvider();
  const reporter = fakeReporter();
  attachWitslogHocuspocus(provider, { report: reporter });

  provider.__emit('close', { event: { code: 1000, reason: '', wasClean: true } });

  assert.strictEqual(reporter.events.length, 0);
  assert.strictEqual(reporter.flushCount, 0);
});

test('disconnect event uses the same abnormal-close filter/capture path', () => {
  const provider = fakeProvider();
  const reporter = fakeReporter();
  attachWitslogHocuspocus(provider, { report: reporter });

  provider.__emit('disconnect', { event: { code: 1006, reason: '', wasClean: false } });

  assert.strictEqual(reporter.events.length, 1);
  assert.strictEqual(reporter.flushCount, 1);
});

test('captures + flushes on authenticationFailed', () => {
  const provider = fakeProvider();
  const reporter = fakeReporter();
  attachWitslogHocuspocus(provider, { report: reporter, tags: ['witsnote'] });

  provider.__emit('authenticationFailed', { reason: 'invalid token' });

  assert.strictEqual(reporter.events.length, 1);
  assert.strictEqual(reporter.events[0].error_code, 'COLLAB_AUTH_FAILED');
  assert.match(reporter.events[0].message, /invalid token/);
  assert.strictEqual(reporter.flushCount, 1);
});

test('works with a plain function report sink (no flush available)', () => {
  const provider = fakeProvider();
  const events = [];
  assert.doesNotThrow(() => {
    attachWitslogHocuspocus(provider, { report: (e) => events.push(e) });
    provider.__emit('close', { event: { code: 1006, reason: '', wasClean: false } });
  });
  assert.strictEqual(events.length, 1);
});

test('detach() unsubscribes all three listeners', () => {
  const provider = fakeProvider();
  const reporter = fakeReporter();
  const detach = attachWitslogHocuspocus(provider, { report: reporter });

  assert.strictEqual(provider.__handlerCount('close'), 1);
  assert.strictEqual(provider.__handlerCount('disconnect'), 1);
  assert.strictEqual(provider.__handlerCount('authenticationFailed'), 1);

  detach();

  assert.strictEqual(provider.__handlerCount('close'), 0);
  assert.strictEqual(provider.__handlerCount('disconnect'), 0);
  assert.strictEqual(provider.__handlerCount('authenticationFailed'), 0);

  provider.__emit('close', { event: { code: 1006, reason: '', wasClean: false } });
  assert.strictEqual(reporter.events.length, 0);
});

test('throws a TypeError when opts.report is missing/invalid', () => {
  const provider = fakeProvider();
  assert.throws(() => attachWitslogHocuspocus(provider, {}), TypeError);
});
