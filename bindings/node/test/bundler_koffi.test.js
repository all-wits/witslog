'use strict';

// Locks the Next.js/webpack bundling bug: koffi's own native `.node` module
// resolution is bundler-incompatible, so any bundler (webpack/turbopack/esbuild/
// Vite SSR) that bundles server code pulling in `koffi` breaks unless `koffi`
// (and `@all-wits/witslog`) are externalized (Next.js: `serverExternalPackages`).
//
// Test A pins the bug reproduction (bundling `require('koffi')` without
// externalizing it fails). Test B proves the documented fix (externalizing
// `koffi`) makes the bundle load fine. If the fix stops working, these fail
// instead of the guidance in the docs going stale.

const { test, after } = require('node:test');
const assert = require('node:assert');
const fs = require('fs');
const path = require('path');
const webpack = require('webpack');

// Fixtures live under bindings/node/.tmp (not os.tmpdir(), and NOT under
// test/ — node's test runner's default glob picks up any .js file inside a
// "test" directory) so webpack's module resolution still walks up to
// bindings/node/node_modules and finds `koffi` the same way real app code
// inside this package tree would.
const TMP_ROOT = path.join(__dirname, '..', '.tmp');

function writeFixture() {
  fs.mkdirSync(TMP_ROOT, { recursive: true });
  const dir = fs.mkdtempSync(path.join(TMP_ROOT, 'bundler-'));
  const entry = path.join(dir, 'entry.js');
  fs.writeFileSync(
    entry,
    "module.exports = require('koffi');\n"
  );
  return { dir, entry };
}

function bundle({ entry, outDir, externals }) {
  return new Promise((resolve, reject) => {
    webpack(
      {
        mode: 'development',
        target: 'node',
        entry,
        externals: externals || {},
        output: { path: outDir, filename: 'bundle.js', libraryTarget: 'commonjs2' },
      },
      (err, stats) => {
        if (err) return reject(err);
        if (stats.hasErrors()) {
          return reject(new Error(stats.toString({ errorDetails: true })));
        }
        resolve(path.join(outDir, 'bundle.js'));
      }
    );
  });
}

function runBundle(bundlePath) {
  // Fresh require so node:test's module cache doesn't mask repeated runs.
  delete require.cache[require.resolve(bundlePath)];
  return require(bundlePath);
}

test('bundling koffi without externalizing it reproduces the Next.js bug', async () => {
  // Plain webpack (no Next.js-style native-module handling) fails to even
  // *parse* koffi's bundled .node binaries as JS, so the failure surfaces at
  // build time here rather than as the runtime "Cannot find the native Koffi
  // module" string Next.js users see (Next's default config tries to load the
  // native module and fails at runtime instead of at bundle time). Either way
  // the root cause is identical — a bundler statically pulling koffi's native
  // addon into the bundle instead of leaving it to `require()` natively — and
  // both failure modes are eliminated the same way: externalizing `koffi`
  // (Test B below), which is exactly what `serverExternalPackages` does.
  const { dir, entry } = writeFixture();
  await assert.rejects(
    () => bundle({ entry, outDir: dir, externals: {} }),
    (e) => /koffi\.node/i.test(e.message) || /Module parse failed/i.test(e.message)
  );
});

test('externalizing koffi (serverExternalPackages equivalent) fixes it', async () => {
  const { dir, entry } = writeFixture();
  const bundlePath = await bundle({ entry, outDir: dir, externals: { koffi: 'commonjs koffi' } });
  const loaded = runBundle(bundlePath);
  assert.strictEqual(typeof loaded.load, 'function');
});

after(() => {
  fs.rmSync(TMP_ROOT, { recursive: true, force: true });
});
