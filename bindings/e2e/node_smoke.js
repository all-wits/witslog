'use strict';

// Node SDK e2e smoke: mount, log (with context+tags), exception, flush.
// Run from inside a `.witslog/` project dir with WITSLOG_LIB set and
// NODE_PATH pointing at bindings/node/node_modules (for koffi).
//
// Usage: node node_smoke.js <marker>

// Resolve the package by absolute path (driver sets WITSLOG_PKG) so the process
// cwd can be the temp project dir (for DB resolution) independent of module lookup.
const witslog = require(process.env.WITSLOG_PKG || 'witslog');

const marker = process.argv[2] || 'NODESMOKE';
const argvMode = process.argv[3] || 'argv-on';
console.log('abi', witslog.ABI_VERSION);

if (argvMode === 'argv-off') {
  // Regression lock: enrich.argv=false must fully suppress argv capture,
  // closing the CLI-arg-secret exposure documented in CONTRACT.md.
  witslog.init({ enrich: { argv: false } });
} else {
  witslog.init();
}

const rowid = witslog.error('node-e2e', `node sdk event ${marker}`, {
  context: { request_id: `${marker}-req`, pid: 1 },
  tags: [marker, 'node', `TAG${marker}`],
  metadata: { lang: 'node' },
});
console.log('rowid', rowid);
if (rowid < 0) {
  console.error('log returned an error');
  process.exit(1);
}

try {
  throw new Error(`boom ${marker}`);
} catch (e) {
  witslog.exception('node-e2e', e, { tags: [marker] });
}

witslog.shutdown();
console.log('NODE_SMOKE_OK', marker);
