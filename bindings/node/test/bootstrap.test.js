'use strict';

const { test } = require('node:test');
const assert = require('node:assert');

const witslog = require('../index');
const { WitslogWriteError } = require('../lib/errors');

test('init with createProject:true bootstraps via witslog_bootstrap_project(null)', () => {
  let bootstrapArg = 'unset';
  let initJson = 'unset';
  witslog.__setLibForTest({
    bootstrapProject: (path) => {
      bootstrapArg = path;
      return 0;
    },
    init: (json) => {
      initJson = json;
      return 0;
    },
  });

  witslog.init({ createProject: true, enrich: { argv: false } });

  assert.strictEqual(bootstrapArg, null);
  assert.ok(initJson.includes('"argv":false'));
  assert.ok(!initJson.includes('createProject'), 'createProject must not leak into the native payload');
});

test('init with createProject:"<path>" forwards the explicit path', () => {
  let bootstrapArg = 'unset';
  witslog.__setLibForTest({
    bootstrapProject: (path) => {
      bootstrapArg = path;
      return 0;
    },
    init: () => 0,
  });

  witslog.init({ createProject: '/tmp/some-project' });

  assert.strictEqual(bootstrapArg, '/tmp/some-project');
});

test('init throws WitslogWriteError when bootstrap fails', () => {
  witslog.__setLibForTest({
    bootstrapProject: () => -1,
    init: () => 0,
  });

  assert.throws(() => witslog.init({ createProject: true }), WitslogWriteError);
});

test('init without createProject never calls bootstrapProject', () => {
  let bootstrapCalled = false;
  witslog.__setLibForTest({
    bootstrapProject: () => {
      bootstrapCalled = true;
      return 0;
    },
    init: () => 0,
  });

  witslog.init();

  assert.strictEqual(bootstrapCalled, false);
});
