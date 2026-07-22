'use strict';

// Next.js adapter — mirrors the express/flask adapter convention: hook the
// framework's own global error signal and attach request context
// automatically, so individual route handlers carry no try/catch +
// witslog.exception boilerplate.
//
//   // instrumentation.ts (Next.js 15+ App Router)
//   import { register, onRequestError } from 'witslog/frameworks/next';
//   register('witsnote-proxy');
//   export { onRequestError };
//
// `onRequestError` is Next's official server-instrumentation error hook —
// it fires for uncaught errors in route handlers, Server Components,
// Server Actions, and middleware alike. Wiring it once via instrumentation.ts
// captures all of them; no per-route try/catch needed.
//
// For Next < 15 (no onRequestError hook), or a single route that wants
// explicit per-call timing/correlation without global instrumentation,
// `withWitslog(handler)` wraps one handler directly.

const witslog = require('../index');
const { checkIngestGuardrails, persistIngestBatch, DEFAULT_RATE_LIMIT } = require('../lib/ingest-core');
const { clampString } = require('../lib/clamp');

let registeredApplication = null;

/**
 * Mount witslog once for this process. Call from the top level of
 * instrumentation.ts (which Next.js guarantees runs once per server
 * instance, before any request is handled) — mirrors express's
 * `witslog.init()` + `witslogErrorHandler(app)` two-step, collapsed into one
 * call since Next has no per-middleware application name to thread through.
 *
 * @param {string} [application]
 * @param {object} [config] - forwarded to witslog.init() (see CONTRACT.md)
 */
function register(application = 'next', config) {
  if (registeredApplication !== null) return;
  witslog.init(config);
  registeredApplication = application;
}

/**
 * Next.js's official server-error hook (Next.js 15+, `experimental.instrumentationHook`
 * or stable per-version — see Next's `instrumentation.ts` docs). Re-export it
 * unchanged:
 *
 *   export { onRequestError } from 'witslog/frameworks/next';
 *
 * Captures the error (including its full `.cause` chain, via index.js's
 * exception()), the request method/path, and Next's router context
 * (routerKind/routePath/routeType/renderSource) — with zero code in the
 * route/component/action that actually failed.
 *
 * @param {unknown} err
 * @param {{ path?: string, method?: string, headers?: Record<string, unknown> }} request
 * @param {{ routerKind?: string, routePath?: string, routeType?: string, renderSource?: string, revalidateReason?: string }} [context]
 */
function onRequestError(err, request, context) {
  const application = registeredApplication || 'next';
  try {
    const error = err instanceof Error ? err : new Error(String(err));
    witslog.exception(application, error, {
      tags: ['next', context && context.routeType].filter(Boolean),
      context: {
        http: { method: request && request.method, path: request && request.path },
        next: context
          ? {
              routerKind: context.routerKind,
              routePath: context.routePath,
              routeType: context.routeType,
              renderSource: context.renderSource,
              revalidateReason: context.revalidateReason,
            }
          : undefined,
      },
    });
  } catch (_e) {
    /* never let logging mask the original request error */
  }
}

/**
 * Higher-order wrapper for a single route handler. Prefer `onRequestError`
 * (global, zero per-route code) when available; use this for Next < 15, or
 * when a route wants explicit timing/correlation capture without relying on
 * global instrumentation.
 *
 * @param {(request: any, ctx: any) => Promise<any>} handler
 * @param {{ application?: string, tags?: string[] }} [opts]
 */
function withWitslog(handler, opts = {}) {
  const { application = registeredApplication || 'next', tags = [] } = opts;
  return async function wrapped(request, ctx) {
    const start = Date.now();
    try {
      return await handler(request, ctx);
    } catch (err) {
      try {
        const error = err instanceof Error ? err : new Error(String(err));
        witslog.exception(application, error, {
          tags: [...tags, 'next', 'route-handler'],
          context: {
            http: {
              method: request && request.method,
              path: request && request.nextUrl ? request.nextUrl.pathname : undefined,
            },
            timing: { latency_ms: Date.now() - start },
          },
        });
      } catch (_e) {
        /* never let logging mask the original error */
      }
      throw err;
    }
  };
}

/**
 * Next.js Route Handler for the browser-ingest endpoint (P10d). Same
 * guardrails/persistence as `witslogBrowserIngest` (frameworks/express.js) —
 * both call the shared framework-neutral core in lib/ingest-core.js — but
 * shaped for the Web Request/Response API Next.js Route Handlers use, not
 * Express's req/res. Express's raw req/res and Next's Request/Response are
 * NOT interchangeable, so this is a real second entry point, not a re-export.
 *
 *   // app/api/__witslog/route.ts
 *   import { witslogNextIngest } from '@all-wits/witslog/frameworks/next';
 *   const handler = witslogNextIngest({ allowedOrigins: ['http://localhost:3000'] });
 *   export { handler as POST };
 *
 * See frameworks/express.js's header comment for the full guardrail
 * rationale (Origin allowlist is the real defense).
 *
 * @param {{
 *   application?: string,
 *   maxBatch?: number,
 *   maxBytes?: number,
 *   allowedOrigins?: string[],
 *   rateLimit?: {windowMs: number, max: number},
 *   force?: boolean,
 * }} options
 * @returns {(request: Request) => Promise<Response>}
 */
function witslogNextIngest(options = {}) {
  const {
    application = 'browser',
    maxBatch = 20,
    maxBytes = 65536,
    allowedOrigins = [],
    rateLimit,
    force = false,
  } = options;

  if (process.env.NODE_ENV === 'production' && !force) {
    throw new Error(
      'witslogNextIngest refuses to arm when NODE_ENV=production — it is an ' +
        'unauthenticated local-dev endpoint that writes attacker-reachable text ' +
        'into the error DB. Pass { force: true } if you have your own auth in front of it.'
    );
  }

  const buckets = new Map();

  return async function handler(request) {
    // Next.js's platform (not this code) enforces its own request-body size
    // limits ahead of this handler ever running; remoteAddress isn't
    // reliably available from the standard Request object behind Next's
    // server runtime, so the loopback check is effectively a no-op here —
    // the Origin allowlist (not loopback) is the real defense regardless
    // (see the guardrail rationale in frameworks/express.js).
    const origin = request.headers.get('origin');
    const rejection = checkIngestGuardrails({
      remoteAddress: undefined,
      origin,
      allowedOrigins,
      rateLimit: rateLimit || DEFAULT_RATE_LIMIT,
      buckets,
    });
    if (rejection) {
      return new Response(rejection.body === undefined ? null : JSON.stringify(rejection.body), {
        status: rejection.status,
        headers: rejection.body === undefined ? undefined : { 'content-type': 'application/json' },
      });
    }

    let rawBody;
    try {
      rawBody = await request.text();
    } catch (_e) {
      return new Response(JSON.stringify({ error: 'invalid body' }), {
        status: 400,
        headers: { 'content-type': 'application/json' },
      });
    }

    const userAgent = clampString(request.headers.get('user-agent'), 500);
    const result = persistIngestBatch(rawBody, { application, maxBatch, maxBytes, userAgent });
    return new Response(result.body === undefined ? null : JSON.stringify(result.body), {
      status: result.status,
      headers: result.body === undefined ? undefined : { 'content-type': 'application/json' },
    });
  };
}

// Test hook: reset module-level registration state between test cases.
function __resetForTest() {
  registeredApplication = null;
}

module.exports = { register, onRequestError, withWitslog, witslogNextIngest, __resetForTest };
