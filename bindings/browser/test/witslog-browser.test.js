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

// --- captureConsole (console.error/warn capture + re-entrancy guard) ---
//
// `init`'s window-dependent branch (event listeners, console patching) only
// runs when `typeof window !== 'undefined'` — there's no real DOM here, so
// these tests stub a minimal `global.window`/`global.fetch` to exercise that
// branch, and always restore/delete globals afterward so later tests in
// this file (and other files sharing the process) aren't affected.

function withStubbedGlobals(fn) {
  const hadWindow = 'window' in global;
  const prevWindow = global.window;
  const hadFetch = 'fetch' in global;
  const prevFetch = global.fetch;

  let capturedBody = null;
  global.window = {
    addEventListener() {},
    removeEventListener() {},
  };
  global.fetch = (_url, opts) => {
    capturedBody = opts && opts.body;
    return Promise.resolve({ ok: true });
  };

  try {
    return fn(() => (capturedBody ? JSON.parse(capturedBody) : null));
  } finally {
    if (hadWindow) global.window = prevWindow;
    else delete global.window;
    if (hadFetch) global.fetch = prevFetch;
    else delete global.fetch;
  }
}

test('captureConsole off by default does not patch console.error/warn', () => {
  withStubbedGlobals(() => {
    const { init } = require('../witslog-browser');
    const originalError = console.error;
    const originalWarn = console.warn;
    const reporter = init({ endpoint: '/ingest' });
    assert.strictEqual(console.error, originalError);
    assert.strictEqual(console.warn, originalWarn);
    reporter._restoreConsole(); // no-op when never patched — must not throw
  });
});

test('captureConsole:true patches console.error, still calls original, and enqueues an event', () => {
  withStubbedGlobals((getBody) => {
    const { init } = require('../witslog-browser');
    const originalError = console.error;
    let originalCalledWith = null;
    console.error = (...args) => {
      originalCalledWith = args;
    };
    const wrappedOriginal = console.error; // captured by init as "original"

    const reporter = init({ endpoint: '/ingest', captureConsole: true });
    assert.notStrictEqual(console.error, wrappedOriginal, 'console.error should now be wrapped');

    console.error('boom', 'detail');
    assert.deepStrictEqual(originalCalledWith, ['boom', 'detail'], 'original console.error must still run');

    reporter.flush();
    const body = getBody();
    assert.ok(body, 'flush should have posted a batch');
    assert.strictEqual(body.events.length, 1);
    assert.match(body.events[0].message, /boom/);
    assert.strictEqual(body.events[0].severity, 'error');
    assert.deepStrictEqual(body.events[0].tags, ['console']);

    reporter._restoreConsole();
    assert.strictEqual(console.error, wrappedOriginal, 'restore should return the exact prior console.error');
    console.error = originalError;
  });
});

test('captureConsole:true patches console.warn at severity warn', () => {
  withStubbedGlobals((getBody) => {
    const { init } = require('../witslog-browser');
    const originalWarn = console.warn;
    console.warn = () => {};
    const reporter = init({ endpoint: '/ingest', captureConsole: true });

    console.warn('careful now');
    reporter.flush();
    const body = getBody();
    assert.strictEqual(body.events[0].severity, 'warn');

    reporter._restoreConsole();
    console.warn = originalWarn;
  });
});

test('captureConsole re-entrancy guard prevents infinite loop when reporting itself logs', () => {
  withStubbedGlobals((getBody) => {
    const { init } = require('../witslog-browser');
    const originalError = console.error;
    let originalCallCount = 0;
    console.error = () => {
      originalCallCount += 1;
    };

    const reporter = init({ endpoint: '/ingest', captureConsole: true });

    // An arg whose JSON serialization re-enters console.error synchronously,
    // while the outer call's own enqueue is still in flight (inReporter
    // guard is active). Without the guard this either infinite-loops or
    // double-enqueues.
    const trap = {
      toJSON() {
        console.error('nested from toJSON');
        return 'trap';
      },
    };

    assert.doesNotThrow(() => console.error('outer', trap));

    // Original console.error ran for BOTH the outer and the nested call —
    // capture must never swallow real developer output, guard or not.
    assert.strictEqual(originalCallCount, 2);

    reporter.flush();
    const body = getBody();
    // Only the outer call's enqueue should have gone through; the nested
    // call's enqueue was skipped by the re-entrancy guard.
    assert.strictEqual(body.events.length, 1);
    assert.match(body.events[0].message, /outer/);

    reporter._restoreConsole();
    console.error = originalError;
  });
});
