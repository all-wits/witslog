'use strict';

// Zero-dep browser error reporter for witslog (P10d). Per PLAN.md §10, no
// native/FFI code ever runs in the browser — this only ships JSON batches to
// a backend endpoint (e.g. `witslogBrowserIngest` in
// bindings/node/frameworks/express.js), which persists them via the Node SDK.
//
// Usage:
//   <script src="witslog-browser.js"></script>
//   <script>
//     WitslogBrowser.init({ endpoint: '/__witslog', app: 'my-web-app' });
//   </script>

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
    context: opts.context,
  };
}

/**
 * Installs `window.onerror` + `unhandledrejection` handlers that batch
 * events and ship them via `navigator.sendBeacon` (survives page unload)
 * with a `fetch(..., {keepalive:true})` fallback, flushing on
 * `visibilitychange`→hidden and `pagehide`.
 *
 * @param {{endpoint: string, app?: string, sampleRate?: number}} config
 * @returns {{flush: Function, enqueue: Function}}
 */
function init(config = {}) {
  const { endpoint, app = 'browser', sampleRate = 1 } = config;
  if (!endpoint) {
    throw new TypeError('endpoint is required');
  }

  let queue = [];

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
    enqueue(
      makeErrorEvent(event.message, {
        stacktrace: event.error && event.error.stack,
        context: { url: event.filename, line: event.lineno, col: event.colno },
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

  if (typeof window !== 'undefined') {
    window.addEventListener('error', onError);
    window.addEventListener('unhandledrejection', onRejection);
    window.addEventListener('pagehide', flush);
    if (typeof document !== 'undefined') {
      document.addEventListener('visibilitychange', () => {
        if (document.visibilityState === 'hidden') flush();
      });
    }
  }

  return { flush, enqueue };
}

const WitslogBrowser = { init, buildBatch, makeErrorEvent };

if (typeof module !== 'undefined' && module.exports) {
  module.exports = WitslogBrowser;
}
if (typeof window !== 'undefined') {
  window.WitslogBrowser = WitslogBrowser;
}
