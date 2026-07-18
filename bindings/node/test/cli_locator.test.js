'use strict';

const { test } = require('node:test');
const assert = require('node:assert');
const fs = require('fs');
const os = require('os');
const path = require('path');

const { binFilename, candidateCliPaths, resolveCliPath } = require('../lib/cli-locator');

function withPlatform(platform, fn) {
  const prev = process.platform;
  Object.defineProperty(process, 'platform', { value: platform, configurable: true });
  try {
    fn();
  } finally {
    Object.defineProperty(process, 'platform', { value: prev, configurable: true });
  }
}

function withEnv(key, value, fn) {
  const prev = process.env[key];
  if (value === undefined) delete process.env[key];
  else process.env[key] = value;
  try {
    fn();
  } finally {
    if (prev === undefined) delete process.env[key];
    else process.env[key] = prev;
  }
}

test('binFilename() is witslog.exe on win32, witslog elsewhere', () => {
  withPlatform('win32', () => assert.strictEqual(binFilename(), 'witslog.exe'));
  withPlatform('linux', () => assert.strictEqual(binFilename(), 'witslog'));
  withPlatform('darwin', () => assert.strictEqual(binFilename(), 'witslog'));
});

test('candidateCliPaths() puts WITSLOG_CLI first, bundled _bin/<platform>/ second, bare name last', () => {
  withEnv('WITSLOG_CLI', '/custom/witslog', () => {
    const paths = candidateCliPaths();
    assert.strictEqual(paths[0], '/custom/witslog');
    assert.ok(paths[1].includes(path.join('_bin')));
    assert.strictEqual(paths[paths.length - 1], binFilename());
  });
});

test('candidateCliPaths() omits WITSLOG_CLI when unset', () => {
  withEnv('WITSLOG_CLI', undefined, () => {
    const paths = candidateCliPaths();
    assert.ok(paths[0].includes(path.join('_bin')));
  });
});

test('resolveCliPath() prefers WITSLOG_CLI when it exists on disk', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'witslog-cli-'));
  const fake = path.join(tmp, 'fake-witslog');
  fs.writeFileSync(fake, '');
  try {
    withEnv('WITSLOG_CLI', fake, () => {
      assert.strictEqual(resolveCliPath(), fake);
    });
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

// Regression lock: when neither WITSLOG_CLI nor the bundled _bin path exist,
// resolveCliPath() must fall through to the bare filename (OS PATH search)
// rather than throwing - this is what lets a separately-installed CLI (the
// pre-bundling workaround) keep working after this change ships.
test('resolveCliPath() falls through to bare filename when nothing bundled exists (regression)', () => {
  withEnv('WITSLOG_CLI', path.join('no', 'such', 'witslog-absent'), () => {
    assert.strictEqual(resolveCliPath(), binFilename());
  });
});

// Regression lock: guards the package.json wiring itself so a future
// package.json edit can't silently drop the npm `bin` entry.
test('package.json declares the witslog bin entry (regression)', () => {
  const pkg = require('../package.json');
  assert.strictEqual(pkg.bin.witslog, 'bin/witslog.js');
});
