'use strict';

const { test } = require('node:test');
const assert = require('node:assert');

const { witslogWebSocketWatch, isAbnormalClose } = require('../witslog-websocket');

test('isAbnormalClose: 1000/1001 are normal, everything else abnormal', () => {
  assert.strictEqual(isAbnormalClose(1000), false);
  assert.strictEqual(isAbnormalClose(1001), false);
  assert.strictEqual(isAbnormalClose(1006), true);
  assert.strictEqual(isAbnormalClose(1011), true);
});

test('requires opts.report', () => {
  assert.throws(() => witslogWebSocketWatch({}), TypeError);
});

test('onClose logs an abnormal close with code/reason/wasClean', () => {
  const events = [];
  const watch = witslogWebSocketWatch({ report: (e) => events.push(e), tags: ['witsnote'] });

  watch.onClose({ event: { code: 1006, reason: 'abnormal', wasClean: false } });

  assert.strictEqual(events.length, 1);
  assert.strictEqual(events[0].error_code, 'WS_CLOSE_1006');
  assert.deepStrictEqual(events[0].context.ws, { code: 1006, reason: 'abnormal', wasClean: false });
  assert.deepStrictEqual(events[0].tags, ['network', 'websocket', 'witsnote']);
});

test('onDisconnect uses the same handler as onClose', () => {
  const events = [];
  const watch = witslogWebSocketWatch({ report: (e) => events.push(e) });
  watch.onDisconnect({ event: { code: 1006, reason: '', wasClean: false } });
  assert.strictEqual(events.length, 1);
});

test('a normal close (1000/1001) is not logged', () => {
  const events = [];
  const watch = witslogWebSocketWatch({ report: (e) => events.push(e) });
  watch.onClose({ event: { code: 1000, reason: '', wasClean: true } });
  watch.onClose({ event: { code: 1001, reason: '', wasClean: true } });
  assert.strictEqual(events.length, 0);
});

test('report as a {enqueue} object works too', () => {
  const enqueued = [];
  const watch = witslogWebSocketWatch({ report: { enqueue: (e) => enqueued.push(e) } });
  watch.onClose({ event: { code: 1006, reason: '', wasClean: false } });
  assert.strictEqual(enqueued.length, 1);
});

test('extra context is merged in', () => {
  const events = [];
  const watch = witslogWebSocketWatch({
    report: (e) => events.push(e),
    context: { board: { boardId: 'b1' } },
  });
  watch.onClose({ event: { code: 1006, reason: '', wasClean: false } });
  assert.deepStrictEqual(events[0].context.board, { boardId: 'b1' });
});
