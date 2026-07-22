'use strict';

// Instrumented fetch — the boilerplate killer for outbound HTTP.
//
// Swap `fetch(url, init)` -> `witslogFetch(url, init, opts)` at your real
// outbound-request choke points (an API client wrapper, a proxy route) and
// every failure / non-2xx response is captured automatically: no more
// hand-written try/catch + witslog.exception/error at each call site.
//
//   const { witslogFetch } = require('witslog/fetch');
//   const res = await witslogFetch(upstreamUrl, fetchInit, { application: 'witsnote-proxy' });
//
// Explicit-wrapper only (decision: no global monkeypatch of `fetch`) — this
// stays safe alongside Next.js's own fetch caching/instrumentation and never
// surprises code that didn't opt in.
//
// Every captured field maps onto an EXISTING witslog_log contract key
// (bindings/CONTRACT.md) — no ABI change. In particular `root_cause` is a
// Rust-only EventBuilder field, not a payload key, so the deepest cause's
// code (unwrapped by index.js's exception()) lands in context.root_cause,
// never top-level — see exception()'s own doc comment in index.js.
//
// Severity policy (decision): expected 4xx (client/caller-caused, e.g. a 409
// optimistic-concurrency conflict) log at 'warn'; 5xx and unreachable/timeout
// log at 'error'. This keeps the error stream meaningful for fingerprinting
// and MTTR instead of flooding it with normal control-flow conflicts.

const { randomUUID } = require('node:crypto');
const witslog = require('./index');
const { clampString } = require('./lib/clamp');

const DEFAULT_CORRELATION_HEADER = 'x-request-id';
const DEFAULT_MAX_BODY_SNAPSHOT = 4096;

function resolveUrl(input) {
  if (typeof input === 'string') return input;
  if (input instanceof URL) return input.toString();
  if (input && typeof input.url === 'string') return input.url; // Request
  return String(input);
}

function resolveMethod(input, init) {
  if (init && init.method) return String(init.method).toUpperCase();
  if (input && typeof input.method === 'string') return input.method.toUpperCase();
  return 'GET';
}

function pathOf(url) {
  try {
    return new URL(url).pathname;
  } catch (_e) {
    return url;
  }
}

/**
 * Inject a correlation-id header for string/URL inputs. A `Request` input
 * already has its headers (and possibly a body stream) baked in;
 * reconstructing it here would risk double-reading a body (see the
 * Turbopack multipart-body gotcha the WitsNote proxy route already works
 * around), so a Request input is passed through unchanged — the correlation
 * id is still minted and logged, just not forwarded as an outbound header.
 */
function withCorrelationHeader(input, init, headerName, correlationId) {
  if (typeof input === 'string' || input instanceof URL) {
    const headers = new Headers((init && init.headers) || undefined);
    if (!headers.has(headerName)) headers.set(headerName, correlationId);
    return { ...(init || {}), headers };
  }
  return init;
}

/** Best-effort peek at a non-2xx response body without consuming it for the caller. */
async function snapshotErrorBody(res, maxLen) {
  try {
    const text = await res.clone().text();
    let parsedError;
    try {
      const parsed = JSON.parse(text);
      // ApiErrorSchema contract shape: { error: { code, message, details } }
      // (see client/lib/api/schemas.ts / server/app/Support/ApiError.php).
      if (parsed && typeof parsed === 'object' && parsed.error) parsedError = parsed.error;
    } catch (_e) {
      /* body isn't JSON, or isn't the {error:{...}} shape — snapshot only */
    }
    return {
      bodySnapshot: clampString(text, maxLen),
      errorCode: parsedError && typeof parsedError.code === 'string' ? parsedError.code : undefined,
      errorMessage:
        parsedError && typeof parsedError.message === 'string' ? parsedError.message : undefined,
      details: parsedError ? parsedError.details : undefined,
    };
  } catch (_e) {
    return {};
  }
}

/**
 * Fetch with automatic witslog capture. Returns the real Response / throws
 * the real error — behaves exactly like `fetch()` to the caller; the
 * response body is untouched (peeked via `.clone()`) so callers can still
 * read it normally.
 *
 * @param {RequestInfo|URL} input
 * @param {RequestInit} [init]
 * @param {{
 *   application?: string,
 *   tags?: string[],
 *   context?: Record<string, unknown>,
 *   correlationId?: string,
 *   correlationHeader?: string,
 *   maxBodySnapshot?: number,
 * }} [opts]
 */
async function witslogFetch(input, init, opts = {}) {
  const {
    application = 'fetch',
    tags = [],
    context: extraContext = {},
    correlationId: suppliedCorrelationId,
    correlationHeader = DEFAULT_CORRELATION_HEADER,
    maxBodySnapshot = DEFAULT_MAX_BODY_SNAPSHOT,
  } = opts;

  const url = resolveUrl(input);
  const method = resolveMethod(input, init);
  const incomingHeader =
    init && init.headers ? new Headers(init.headers).get(correlationHeader) : null;
  const correlationId = suppliedCorrelationId || incomingHeader || randomUUID();

  const finalInit = withCorrelationHeader(input, init, correlationHeader, correlationId);

  const start = Date.now();
  let res;
  try {
    res = await fetch(input, finalInit);
  } catch (err) {
    const latencyMs = Date.now() - start;
    try {
      witslog.exception(application, err, {
        correlation_id: correlationId,
        error_code: 'UPSTREAM_UNREACHABLE',
        tags: [...tags, 'fetch', 'upstream-unreachable'],
        context: {
          ...extraContext,
          http: { method, url, path: pathOf(url) },
          timing: { latency_ms: latencyMs },
        },
      });
    } catch (_e) {
      /* never let logging mask the original fetch failure */
    }
    throw err;
  }
  const latencyMs = Date.now() - start;

  if (res.status >= 400) {
    try {
      const { bodySnapshot, errorCode, errorMessage, details } = await snapshotErrorBody(
        res,
        maxBodySnapshot
      );
      const severity = res.status >= 500 ? 'error' : 'warn';
      const isRetryable = res.status >= 500 || res.status === 408 || res.status === 429;
      const summary = errorMessage ? `${method} ${pathOf(url)} → ${res.status} (${errorMessage})`
        : `${method} ${pathOf(url)} → ${res.status}`;
      witslog.log(application, summary, {
        severity,
        correlation_id: correlationId,
        error_code: errorCode || `HTTP_${res.status}`,
        tags: [
          ...tags,
          'fetch',
          severity === 'error' ? 'upstream-5xx' : 'upstream-4xx',
          ...(isRetryable ? ['retryable'] : []),
        ],
        context: {
          ...extraContext,
          http: { method, url, path: pathOf(url), status: res.status },
          timing: { latency_ms: latencyMs },
          upstream:
            bodySnapshot !== undefined
              ? { body: bodySnapshot, error_code: errorCode, error_message: errorMessage, details }
              : undefined,
        },
      });
    } catch (_e) {
      /* never let logging mask the response for the caller */
    }
  }

  return res;
}

module.exports = { witslogFetch };
