'use strict';

// Packaged copy of ../browser/witslog-browser.js, published as the
// `@all-wits/witslog/browser` subpath (added in 0.6.1 — package.json's
// `files` array can only include paths under bindings/node/, so the
// canonical file at bindings/browser/witslog-browser.js can't be referenced
// directly from the npm tarball). Keep byte-identical to that file; the
// canonical copy remains the source of truth for standalone `<script src>`
// usage documented in bindings/CONTRACT.md and the repo READMEs.
//
// Zero-dep browser error reporter for witslog (P10d). Per PLAN.md §10, no
// native/FFI code ever runs in the browser — this only ships JSON batches to
// a backend endpoint (e.g. `witslogBrowserIngest` in
// bindings/node/frameworks/express.js, or `witslogNextIngest` in
// bindings/node/frameworks/next.js), which persists them via the Node SDK.
//
// Usage:
//   import WitslogBrowser from '@all-wits/witslog/browser';
//   WitslogBrowser.init({ endpoint: '/api/witslog-ingest', app: 'my-web-app' });
//
// By default this only captures uncaught throws (`window.onerror`) and
// unhandled promise rejections — most DevTools "red" console lines are
// `console.error`/`console.warn` calls that never throw (React caught-error
// logs, prop/hydration warnings, third-party libs) and are invisible to
// those two hooks. Pass `captureConsole: true` to also capture those (see
// `init` below) — opt-in because it patches a global and can be noisy.

/** Pure — builds the ingest batch body. Unit-testable without a DOM. */
function buildBatch(events, meta) {
  return {
    application: meta.app,
    events: events.map((e) => ({
      message: e.message,
      severity: e.severity || 'error',
      exception: e.exception,
      stacktrace: e.stacktrace,
      error_code: e.error_code,
      correlation_id: e.correlation_id,
      tags: e.tags,
      context: e.context,
    })),
  };
}

/** Pure — normalizes a raw browser error into the batch event shape. */
function makeErrorEvent(message, opts = {}) {
  return {
    message: String(message == null ? 'error' : message),
    severity: opts.severity || 'error',
    exception: opts.exception,
    stacktrace: opts.stacktrace,
    error_code: opts.error_code,
    tags: opts.tags,
    context: opts.context,
  };
}

/** Pure — best-effort stringification of a single console.error/warn arg. */
function stringifyConsoleArg(arg) {
  if (typeof arg === 'string') return arg;
  if (arg instanceof Error) return arg.message || String(arg);
  try {
    return JSON.stringify(arg);
  } catch (_e) {
    return String(arg);
  }
}

/** Pure — joins console.error/warn args into one message string. */
function formatConsoleArgs(args) {
  return Array.prototype.map.call(args, stringifyConsoleArg).join(' ');
}

/**
 * Installs `window.onerror` + `unhandledrejection` handlers (plus, when
 * enabled, `console.error`/`console.warn` patching and capture-phase
 * resource-load error capture) that batch events and ship them via
 * `navigator.sendBeacon` (survives page unload) with a
 * `fetch(..., {keepalive:true})` fallback, flushing on
 * `visibilitychange`→hidden and `pagehide`.
 *
 * @param {{
 *   endpoint: string,
 *   app?: string,
 *   sampleRate?: number,
 *   captureConsole?: boolean,
 * }} config `captureConsole` (default false) also captures `console.error`
 *   (severity `error`) and `console.warn` (severity `warn`) calls, tagged
 *   `['console']`, and resource-load failures (`<img>`/`<script>`/`<link>`
 *   404s etc, tagged `['resource']`) which normally only fire a
 *   non-bubbling `error` event `window.onerror` never sees.
 * @returns {{flush: Function, enqueue: Function}}
 */
function init(config = {}) {
  const { endpoint, app = 'browser', sampleRate = 1, captureConsole = false } = config;
  if (!endpoint) {
    throw new TypeError('endpoint is required');
  }

  let queue = [];
  // Re-entrancy guard: reporting a captured console.error must never itself
  // call console.error/warn (directly, or indirectly via a failed fetch/
  // sendBeacon logging through console) and recurse back into the patched
  // methods — that would loop forever the moment reporting itself errors.
  let inReporter = false;

  function shouldSample() {
    return sampleRate >= 1 || Math.random() < sampleRate;
  }

  function enqueue(event) {
    if (!shouldSample()) return;
    queue.push(event);
  }

  function flush() {
    if (queue.length === 0) return;
    const batch = buildBatch(queue, { app });
    queue = [];
    const body = JSON.stringify(batch);

    if (typeof navigator !== 'undefined' && typeof navigator.sendBeacon === 'function') {
      const blob = new Blob([body], { type: 'application/json' });
      if (navigator.sendBeacon(endpoint, blob)) {
        return;
      }
    }
    if (typeof fetch === 'function') {
      fetch(endpoint, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body,
        keepalive: true,
      }).catch(() => {
        /* best-effort — never let reporting failures surface to the app */
      });
    }
  }

  function onError(event) {
    // Script errors dispatch directly at Window (event.target === window),
    // invoking this bubble-registered listener once — unaffected by the
    // separate capture-phase resource listener below (different function,
    // different purpose, so no double-enqueue).
    enqueue(
      makeErrorEvent(event.message, {
        stacktrace: event.error && event.error.stack,
        context: { url: event.filename, line: event.lineno, col: event.colno },
      })
    );
  }

  /**
   * Capture-phase only. Resource-load failures (img/script/link 404s etc.)
   * dispatch a non-bubbling `error` event targeted at the failed element —
   * `window`'s bubble-phase listener (`onError` above) never sees it; only a
   * capture-phase listener on an ancestor (window) observes it as the event
   * travels down to its target. Script errors are dispatched AT window
   * (event.target === window) and are already handled by `onError` above —
   * skip those here so they aren't enqueued twice.
   */
  function onResourceError(event) {
    if (event.target === window) return;
    const el = event.target;
    if (!el) return;
    const src = el.src || el.href || '';
    enqueue(
      makeErrorEvent('resource load failed', {
        tags: ['resource'],
        context: { url: src, tag: el.tagName && el.tagName.toLowerCase() },
      })
    );
  }

  function onRejection(event) {
    const reason = event.reason;
    const message = reason && reason.message ? reason.message : reason;
    enqueue(
      makeErrorEvent(message, {
        exception: 'UnhandledRejection',
        stacktrace: reason && reason.stack,
      })
    );
  }

  let restoreConsole = null;

  function patchConsole() {
    if (typeof console === 'undefined') return null;
    const originalError = console.error;
    const originalWarn = console.warn;
    if (typeof originalError !== 'function' && typeof originalWarn !== 'function') return null;

    function wrap(original, severity) {
      if (typeof original !== 'function') return original;
      return function patched(...args) {
        // Always call the original first — never swallow developer output,
        // even if reporting below throws or is skipped by the guard.
        original.apply(console, args);
        if (inReporter) return;
        inReporter = true;
        try {
          const firstError = args.find((a) => a instanceof Error);
          enqueue(
            makeErrorEvent(formatConsoleArgs(args), {
              severity,
              exception: firstError ? firstError.name : undefined,
              stacktrace: firstError && firstError.stack,
              tags: ['console'],
            })
          );
        } catch (_e) {
          /* never let capture itself throw into caller's console.error call */
        } finally {
          inReporter = false;
        }
      };
    }

    console.error = wrap(originalError, 'error');
    console.warn = wrap(originalWarn, 'warn');

    return function restore() {
      console.error = originalError;
      console.warn = originalWarn;
    };
  }

  if (typeof window !== 'undefined') {
    window.addEventListener('error', onError);
    window.addEventListener('unhandledrejection', onRejection);
    window.addEventListener('pagehide', flush);
    if (typeof document !== 'undefined') {
      document.addEventListener('visibilitychange', () => {
        if (document.visibilityState === 'hidden') flush();
      });
    }
    if (captureConsole) {
      restoreConsole = patchConsole();
      // Resource-load errors (img/script/link) don't bubble — only a
      // capture-phase listener observes them. Bundled under the same
      // opt-in as console capture (both add new capture volume beyond the
      // pre-existing default of uncaught-throw + unhandled-rejection only).
      window.addEventListener('error', onResourceError, true);
    }
  }

  return {
    flush,
    enqueue,
    /** Test/cleanup hook — undoes console patching + the capture-phase
     * resource listener if `captureConsole` was on. */
    _restoreConsole: () => {
      if (restoreConsole) restoreConsole();
      if (typeof window !== 'undefined') {
        window.removeEventListener('error', onResourceError, true);
      }
    },
  };
}

const WitslogBrowser = { init, buildBatch, makeErrorEvent };

if (typeof module !== 'undefined' && module.exports) {
  module.exports = WitslogBrowser;
}
if (typeof window !== 'undefined') {
  window.WitslogBrowser = WitslogBrowser;
}
