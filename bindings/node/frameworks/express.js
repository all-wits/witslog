'use strict';

// Express adapter. Mount witslog at startup, then register this AFTER your routes:
//
//   const witslog = require('witslog');
//   const { witslogErrorHandler } = require('witslog/frameworks/express');
//   witslog.init();
//   app.use(witslogErrorHandler('myapp'));   // last, after routes
//
// Logs any error passed to next(err), then forwards it to the next handler.

const witslog = require('../index');
const { clampString } = require('../lib/clamp');
const {
  DEFAULT_RATE_LIMIT,
  isLoopback,
  clampSeverity,
  checkIngestGuardrails,
  persistIngestBatch,
} = require('../lib/ingest-core');

function witslogErrorHandler(application = 'express') {
  return function (err, req, res, next) {
    try {
      witslog.exception(application, err, {
        context: { path: req && req.path, method: req && req.method },
      });
    } catch (_e) {
      /* never let logging mask the request error */
    }
    next(err);
  };
}

// ---------------------------------------------------------------------------
// P10d: browser-side error ingest (client-runtime capture, not HTTP 4xx).
//
// The body of this endpoint is UNTRUSTED INPUT: text posted here lands in
// events.message, which search_errors/explain_error return verbatim to an
// MCP-connected LLM (see PLAN.md §10 and bindings/CONTRACT.md). Clamping
// severity is NOT the defense against that — the message text is the
// payload, not the severity. The real defenses, all required together:
//
//   1. Origin allowlist, fail-closed (default []) — the sharp attack is a
//      malicious page open in the SAME browser as your dev server, doing a
//      same-machine cross-origin POST to localhost. A loopback/remoteAddress
//      check alone does NOT stop this (the request genuinely originates from
//      127.0.0.1); only the Origin header check does.
//   2. Bind the HOST SERVER to 127.0.0.1 yourself — this middleware cannot
//      control your `app.listen()` call, so it is your responsibility. The
//      remoteAddress check below is defense-in-depth only, for the case
//      where the app is mistakenly reachable from other interfaces.
//   3. Refuses to arm when NODE_ENV=production unless explicitly forced.
//   4. Rate limiting — per-request size/batch caps do nothing against
//      request VOLUME (20 events x 64KB is nothing at 10k req/s); the real
//      risk is unbounded local disk growth + FTS5 index blowup.
//   5. Severity clamp (error|warn only, never fatal/critical) + message/
//      stacktrace length caps + batch/body byte caps, as defense-in-depth
//      once the above hold.
//
// `tags:['browser']` is provenance-flavored but NOT a trust boundary —
// classify() merges suggested tags into existing ones, so tags are advisory
// metadata, not cryptographic origin proof. True provenance (`ingest_source`
// in the payload contract) needs an ABI-version bump and is out of scope here.

/**
 * Express handler that accepts batches from `bindings/browser/witslog-browser.js`
 * and persists them via the Node SDK. See the guardrail notes above — every
 * option here defaults to the fail-closed/dev-only posture; loosen deliberately.
 *
 * @param {{
 *   application?: string,
 *   path?: string,
 *   maxBatch?: number,
 *   maxBytes?: number,
 *   allowedOrigins?: string[],
 *   rateLimit?: {windowMs: number, max: number},
 *   force?: boolean,
 * }} options
 */
function witslogBrowserIngest(options = {}) {
  const {
    application = 'browser',
    path = '/__witslog',
    maxBatch = 20,
    maxBytes = 65536,
    allowedOrigins = [],
    rateLimit = DEFAULT_RATE_LIMIT,
    force = false,
  } = options;

  if (process.env.NODE_ENV === 'production' && !force) {
    throw new Error(
      'witslogBrowserIngest refuses to arm when NODE_ENV=production — it is an ' +
        'unauthenticated local-dev endpoint that writes attacker-reachable text ' +
        'into the error DB. Pass { force: true } if you have your own auth in front of it.'
    );
  }

  const buckets = new Map();

  return function (req, res, next) {
    if (req.path !== path) {
      next();
      return;
    }

    const remote = req.socket && req.socket.remoteAddress;
    const origin = req.get && req.get('origin');
    const rejection = checkIngestGuardrails({ remoteAddress: remote, origin, allowedOrigins, rateLimit, buckets });
    if (rejection) {
      if (rejection.body === undefined) res.status(rejection.status).end();
      else res.status(rejection.status).json(rejection.body);
      return;
    }

    let raw = '';
    let responded = false;
    // `req.destroy()` may prevent 'end' from ever firing, so the oversize
    // response is sent immediately here rather than deferred to 'end' —
    // deferring it would leave the client hanging with no response. This
    // mid-stream abort is Express/raw-Node-stream-specific defense-in-depth;
    // persistIngestBatch (lib/ingest-core.js) also enforces maxBytes as a
    // post-buffer check, which is all a fully-buffered transport (e.g. the
    // Next.js ingest handler in frameworks/next.js) can do.
    function respondOnce(status, jsonBody) {
      if (responded) return;
      responded = true;
      if (jsonBody === undefined) res.status(status).end();
      else res.status(status).json(jsonBody);
    }

    req.on('data', (chunk) => {
      if (responded) return;
      raw += chunk;
      if (raw.length > maxBytes) {
        respondOnce(413, { error: 'payload too large' });
        req.destroy();
      }
    });
    req.on('end', () => {
      if (responded) return;
      const userAgent = clampString(req.get && req.get('user-agent'), 500);
      const result = persistIngestBatch(raw, { application, maxBatch, maxBytes, userAgent });
      respondOnce(result.status, result.body);
    });
  };
}

module.exports = {
  witslogErrorHandler,
  witslogBrowserIngest,
  // Test hooks — pure guardrail helpers, exported so they're unit-testable
  // without needing a real express req/res.
  __isLoopback: isLoopback,
  __clampString: clampString,
  __clampSeverity: clampSeverity,
};
