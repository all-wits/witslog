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

const DEFAULT_RATE_LIMIT = { windowMs: 60_000, max: 60 };

function isLoopback(addr) {
  return addr === '127.0.0.1' || addr === '::1' || addr === '::ffff:127.0.0.1';
}

function clampString(value, maxLen) {
  if (typeof value !== 'string') return undefined;
  return value.length > maxLen ? value.slice(0, maxLen) : value;
}

function clampSeverity(sev) {
  // Untrusted input never gets to claim fatal/critical.
  return sev === 'warn' ? 'warn' : 'error';
}

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

  function isRateLimited(key) {
    const now = Date.now();
    let bucket = buckets.get(key);
    if (!bucket || now >= bucket.resetAt) {
      bucket = { count: 0, resetAt: now + rateLimit.windowMs };
      buckets.set(key, bucket);
    }
    bucket.count += 1;
    return bucket.count > rateLimit.max;
  }

  return function (req, res, next) {
    if (req.path !== path) {
      next();
      return;
    }

    const remote = req.socket && req.socket.remoteAddress;
    if (remote && !isLoopback(remote)) {
      res.status(403).end();
      return;
    }

    const origin = req.get && req.get('origin');
    if (!origin || !allowedOrigins.includes(origin)) {
      res.status(403).json({ error: 'origin not allowed' });
      return;
    }

    if (isRateLimited(remote || origin)) {
      res.status(429).json({ error: 'rate limit exceeded' });
      return;
    }

    let raw = '';
    let responded = false;
    // `req.destroy()` may prevent 'end' from ever firing, so the oversize
    // response is sent immediately here rather than deferred to 'end' —
    // deferring it would leave the client hanging with no response.
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

      let body;
      try {
        body = JSON.parse(raw);
      } catch (_e) {
        respondOnce(400, { error: 'invalid JSON' });
        return;
      }

      const events = Array.isArray(body.events) ? body.events.slice(0, maxBatch) : [];
      const userAgent = clampString(req.get && req.get('user-agent'), 500);

      for (const evt of events) {
        try {
          witslog.log(application, clampString(evt && evt.message, 2000) || 'browser error', {
            severity: clampSeverity(evt && evt.severity),
            exception: clampString(evt && evt.exception, 200),
            stacktrace: clampString(evt && evt.stacktrace, 8000),
            tags: ['browser'],
            context: {
              url: clampString(evt && evt.context && evt.context.url, 2000),
              ua: userAgent,
            },
          });
        } catch (_e) {
          /* one malformed event must not break the batch or the response */
        }
      }

      respondOnce(202);
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
