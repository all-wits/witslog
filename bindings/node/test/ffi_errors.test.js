'use strict';

const { test } = require('node:test');
const assert = require('node:assert');

const ffi = require('../lib/ffi');
const witslog = require('../index');
const {
  WitslogLibraryError,
  WitslogContractError,
  WitslogWriteError,
} = require('../lib/errors');

test('missing library raises WitslogLibraryError listing paths', () => {
  const prev = process.env.WITSLOG_LIB;
  const bogus = require('path').join('no', 'such', 'witslog_ffi_absent.dll');
  process.env.WITSLOG_LIB = bogus;
  try {
    assert.throws(
      () => ffi.loadLibrary(),
      (e) => e instanceof WitslogLibraryError && e.searchedPaths.includes(bogus)
    );
  } finally {
    if (prev === undefined) delete process.env.WITSLOG_LIB;
    else process.env.WITSLOG_LIB = prev;
  }
});

test('checkAbi throws WitslogContractError on mismatch', () => {
  assert.throws(() => ffi.checkAbi(999, 1), (e) => e instanceof WitslogContractError && e.actual === 999);
});

test('checkAbi passes on match', () => {
  assert.doesNotThrow(() => ffi.checkAbi(1, 1));
});

test('write error on negative return', () => {
  witslog.__setLibForTest({ log: () => -1 });
  assert.throws(() => witslog.log('app', 'boom'), WitslogWriteError);
});

test('log returns rowid via fake lib', () => {
  witslog.__setLibForTest({ log: () => 42 });
  assert.strictEqual(witslog.log('app', 'ok', { context: { a: 1 } }), 42);
});

test('init forwards argv-disable config unchanged (regression lock)', () => {
  // The suppression itself is proven natively (witslog-ffi::
  // configure_argv_false_suppresses_argv_capture); this locks that the Node
  // `init()` surface forwards the config unchanged, so an app that may pass
  // secrets as bare CLI args can fully close that exposure.
  let captured = null;
  witslog.__setLibForTest({
    init: (json) => {
      captured = json;
      return 0;
    },
  });
  witslog.init({ enrich: { argv: false } });
  assert.ok(captured.includes('"argv":false'));
});

test('exception captures stack via fake lib', () => {
  let captured = null;
  witslog.__setLibForTest({ log: (json) => ((captured = json), 1) });
  witslog.exception('app', new Error('kaboom'));
  assert.ok(captured.includes('kaboom'));
  assert.ok(captured.includes('stacktrace'));
  assert.ok(captured.includes('Error'));
});
