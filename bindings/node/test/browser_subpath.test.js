'use strict';

// Regression lock: bindings/node/browser.js (published as the
// `@all-wits/witslog/browser` npm subpath, 0.6.1+) must stay in lockstep
// with the canonical bindings/browser/witslog-browser.js — the header of
// each file commits to this, but nothing previously enforced it. Compares
// on the functional-source body (strips each file's own header comment
// block, which legitimately differs — the packaged copy documents the
// import-path usage, the canonical file documents <script src> usage) so a
// real behavioral drift fails this test even though the intro comments
// differ on purpose.

const { test } = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

function stripLeadingCommentBlock(src) {
  const lines = src.split('\n');
  let i = 0;
  // Skip the 'use strict'; line.
  if (lines[i] && lines[i].trim() === "'use strict';") i += 1;
  // Skip a leading run of blank lines and `//` comment lines.
  while (i < lines.length && (lines[i].trim() === '' || lines[i].trim().startsWith('//'))) {
    i += 1;
  }
  return lines.slice(i).join('\n');
}

test('packaged browser.js body matches canonical bindings/browser/witslog-browser.js', () => {
  const canonicalPath = path.join(__dirname, '..', '..', 'browser', 'witslog-browser.js');
  const packagedPath = path.join(__dirname, '..', 'browser.js');

  const canonical = stripLeadingCommentBlock(fs.readFileSync(canonicalPath, 'utf8'));
  const packaged = stripLeadingCommentBlock(fs.readFileSync(packagedPath, 'utf8'));

  assert.strictEqual(
    packaged,
    canonical,
    'bindings/node/browser.js has drifted from bindings/browser/witslog-browser.js — ' +
      'apply the same change to both (see each file\'s header comment).'
  );
});

test('packaged browser.js exports the same API shape as the canonical file', () => {
  const canonical = require('../../browser/witslog-browser.js');
  const packaged = require('../browser.js');

  assert.deepStrictEqual(Object.keys(packaged).sort(), Object.keys(canonical).sort());
  for (const key of Object.keys(canonical)) {
    assert.strictEqual(typeof packaged[key], typeof canonical[key], `export '${key}' type mismatch`);
  }
});
