'use strict';

// Framework-neutral core for the browser-ingest endpoint (P10d): the
// guardrail decisions (loopback/origin/rate-limit) and the actual batch
// persistence, with NO transport I/O (no req/res, no streaming) — so every
// transport-specific wrapper (Express's raw req/res streaming in
// frameworks/express.js, Next.js's Request/Response Web API in
// frameworks/next.js) calls the SAME logic instead of re-implementing (and
// risking drifting on) the security-relevant guardrails documented in
// frameworks/express.js's header comment.
//
// See that file for the full guardrail rationale (Origin allowlist is the
// real defense, not the loopback check — a same-machine malicious page's
// POST to localhost genuinely originates from 127.0.0.1).

const witslog = require('../index');
const { clampString, clampContext } = require('./clamp');

const DEFAULT_RATE_LIMIT = { windowMs: 60_000, max: 60 };

function isLoopback(addr) {
  return addr === '127.0.0.1' || addr === '::1' || addr === '::ffff:127.0.0.1';
}

function clampSeverity(sev) {
  // Untrusted input never gets to claim fatal/critical.
  return sev === 'warn' ? 'warn' : 'error';
}

/**
 * Loopback / Origin-allowlist / rate-limit checks, in the same order and
 * with the same status codes as the original Express-only implementation.
 * Returns `null` when the request may proceed, or `{status, body?}` to send
 * immediately and stop.
 *
 * @param {{ remoteAddress?: string, origin?: string, allowedOrigins: string[], rateLimit: {windowMs:number,max:number}, buckets: Map }} ctx
 */
function checkIngestGuardrails({ remoteAddress, origin, allowedOrigins, rateLimit, buckets }) {
  if (remoteAddress && !isLoopback(remoteAddress)) {
    return { status: 403 };
  }
  if (!origin || !allowedOrigins.includes(origin)) {
    return { status: 403, body: { error: 'origin not allowed' } };
  }

  const key = remoteAddress || origin;
  const now = Date.now();
  let bucket = buckets.get(key);
  if (!bucket || now >= bucket.resetAt) {
    bucket = { count: 0, resetAt: now + rateLimit.windowMs };
    buckets.set(key, bucket);
  }
  bucket.count += 1;
  if (bucket.count > rateLimit.max) {
    return { status: 429, body: { error: 'rate limit exceeded' } };
  }

  return null;
}

/**
 * Parse + persist an already-fully-buffered ingest request body. Byte-size
 * enforcement here is a post-buffer check (`rawBody.length > maxBytes`) —
 * Express's own wrapper additionally aborts mid-stream on an oversized body
 * (see frameworks/express.js) since it has access to the raw chunked
 * stream; a transport that only exposes a fully-buffered body (e.g. Next.js
 * Route Handlers via `request.text()`) can only check after the fact, which
 * is an acceptable relaxation — Next itself already enforces its own
 * request body size limits ahead of this code ever running.
 *
 * @param {string} rawBody
 * @param {{ application: string, maxBatch: number, maxBytes: number, userAgent?: string }} opts
 */
function persistIngestBatch(rawBody, { application, maxBatch, maxBytes, userAgent }) {
  if (rawBody.length > maxBytes) {
    return { status: 413, body: { error: 'payload too large' } };
  }

  let parsed;
  try {
    parsed = JSON.parse(rawBody);
  } catch (_e) {
    return { status: 400, body: { error: 'invalid JSON' } };
  }

  const events = Array.isArray(parsed.events) ? parsed.events.slice(0, maxBatch) : [];

  for (const evt of events) {
    try {
      // Beyond message/exception/stacktrace, forward the event's `context`
      // object (clamped: bounded depth/keys/string length — see
      // clampContext) rather than only context.url as in the pre-clampContext
      // implementation. This is what lets frameworks/react-query.js's
      // captured mutation variables/response reach storage instead of being
      // silently dropped at the ingest boundary. `tags` stays
      // untrusted-input-safe: 'browser' is always first and cannot be
      // removed, at most a few additional low-cardinality client tags are
      // appended.
      const extraTags = Array.isArray(evt && evt.tags)
        ? evt.tags
            .filter((t) => typeof t === 'string')
            .slice(0, 5)
            .map((t) => clampString(t, 50))
        : [];
      witslog.log(application, clampString(evt && evt.message, 2000) || 'browser error', {
        severity: clampSeverity(evt && evt.severity),
        exception: clampString(evt && evt.exception, 200),
        stacktrace: clampString(evt && evt.stacktrace, 8000),
        error_code: clampString(evt && evt.error_code, 100),
        tags: ['browser', ...extraTags],
        context: {
          ...clampContext(evt && evt.context),
          url: clampString(evt && evt.context && evt.context.url, 2000),
          ua: userAgent,
        },
      });
    } catch (_e) {
      /* one malformed event must not break the batch or the response */
    }
  }

  return { status: 202 };
}

module.exports = {
  DEFAULT_RATE_LIMIT,
  isLoopback,
  clampSeverity,
  checkIngestGuardrails,
  persistIngestBatch,
};
