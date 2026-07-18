'use strict';

const { test } = require('node:test');
const assert = require('node:assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');

const SHIM = path.join(__dirname, '..', 'bin', 'witslog.js');

function withEnv(overrides, fn) {
  const prev = {};
  for (const k of Object.keys(overrides)) prev[k] = process.env[k];
  Object.assign(process.env, overrides);
  try {
    return fn();
  } finally {
    for (const k of Object.keys(overrides)) {
      if (prev[k] === undefined) delete process.env[k];
      else process.env[k] = prev[k];
    }
  }
}

test('bin/witslog.js forwards argv and exit code to the resolved CLI', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'witslog-shim-'));
  const stub = path.join(tmp, 'stub.js');
  // WITSLOG_CLI=process.execPath makes node itself the "CLI"; the stub script
  // (forwarded as an argv element) is what proves argv/stdio/exit code cross
  // the shim's spawnSync unchanged - no compiled Rust binary needed here.
  fs.writeFileSync(
    stub,
    "console.log('ok ' + process.argv.slice(2).join(' ')); process.exit(7);"
  );
  try {
    const result = withEnv({ WITSLOG_CLI: process.execPath }, () =>
      spawnSync(process.execPath, [SHIM, stub, 'query', 'marker'], { encoding: 'utf8' })
    );
    assert.strictEqual(result.status, 7);
    assert.match(result.stdout, /^ok .*query marker/);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('bin/witslog.js reports WitslogCliNotFoundError when no candidate resolves (ENOENT)', () => {
  // A nonexistent WITSLOG_CLI and no bundled _bin/ still leave resolveCliPath()
  // falling through to the bare 'witslog' filename (by design - see the
  // regression test in cli_locator.test.js); this asserts the *other* half of
  // that contract: when the OS then can't find 'witslog' on PATH either
  // (true in this sandboxed test env), the shim's spawn-time ENOENT handler
  // surfaces a clear WitslogCliNotFoundError instead of a raw ENOENT.
  const missing = path.join(os.tmpdir(), 'witslog-shim-does-not-exist', 'witslog-absent');
  const result = withEnv({ WITSLOG_CLI: missing, PATH: '' }, () =>
    spawnSync(process.execPath, [SHIM, 'query'], { encoding: 'utf8' })
  );
  assert.strictEqual(result.status, 1);
  assert.match(result.stderr, /could not locate the witslog CLI binary/);
  assert.match(result.stderr, /WITSLOG_CLI/);
});
