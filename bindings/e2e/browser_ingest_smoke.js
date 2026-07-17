'use strict';

// P10 e2e smoke: proves a browser-shaped POST crosses
// witslogBrowserIngest -> real native FFI (not a fake lib) -> real DB, and
// is readable back through the real CLI (driven by run.ps1's Gate 4).
//
// Usage: node browser_ingest_smoke.js <marker>
// Requires WITSLOG_PKG (path to bindings/node/index.js) and WITSLOG_LIB
// (path to the built witslog_ffi dll/so) set by the caller, same as
// node_smoke.js.

const http = require('node:http');

const marker = process.argv[2];
if (!marker) {
  console.error('usage: node browser_ingest_smoke.js <marker>');
  process.exit(1);
}

const witslog = require(process.env.WITSLOG_PKG);
const { witslogBrowserIngest } = require(require('path').join(
  require('path').dirname(process.env.WITSLOG_PKG),
  'frameworks',
  'express.js'
));

witslog.init();

const ingest = witslogBrowserIngest({
  force: true,
  allowedOrigins: ['http://localhost:5173'],
  application: 'browser-e2e',
});

const server = http.createServer((req, res) => {
  req.path = req.url;
  req.get = (name) => req.headers[name.toLowerCase()];
  res.status = function (code) {
    this.statusCode = code;
    return this;
  };
  res.json = function (body) {
    this.end(JSON.stringify(body));
  };
  ingest(req, res, () => {
    res.statusCode = 404;
    res.end();
  });
});

server.listen(0, '127.0.0.1', () => {
  const port = server.address().port;
  const body = JSON.stringify({ events: [{ message: `${marker} boom`, severity: 'error' }] });

  const req = http.request(
    {
      hostname: '127.0.0.1',
      port,
      path: '/__witslog',
      method: 'POST',
      headers: {
        origin: 'http://localhost:5173',
        'content-type': 'application/json',
        'content-length': Buffer.byteLength(body),
      },
    },
    (res) => {
      let ok = res.statusCode === 202;
      res.on('data', () => {});
      res.on('end', () => {
        server.close();
        witslog.flush();
        if (!ok) {
          console.error(`ingest POST failed: status=${res.statusCode}`);
          process.exit(1);
        }
        console.log(`browser_ingest_smoke: posted marker "${marker}" via real FFI, status=${res.statusCode}`);
        process.exit(0);
      });
    }
  );
  req.on('error', (e) => {
    console.error('request error:', e);
    process.exit(1);
  });
  req.write(body);
  req.end();
});
