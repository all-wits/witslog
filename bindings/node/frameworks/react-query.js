'use strict';

// React Query global capture — the "TanStack Devtools, but persisted to
// witslog" layer. Subscribes to a QueryClient's MutationCache/QueryCache
// (the same event stream TanStack Query Devtools itself observes) so every
// query/mutation failure is captured automatically — the mutation/query
// key, the request payload (variables), and the error — with zero code in
// individual hooks.
//
// This closes the gap left by WitsNote's own `registerMutationDefaults`
// (client/lib/api/hooks.ts): its per-mutation `onError` callbacks only do
// optimistic-update rollback and never log anything, so client-side
// query/mutation failures were captured nowhere before this module.
//
// Browser-safe: no Node built-ins, no FFI, no hard @tanstack/react-query
// dependency (duck-typed against `.getMutationCache()`/`.getQueryCache()`,
// each exposing `.subscribe(listener) -> unsubscribe`, which is the same
// public Cache API TanStack Query Devtools itself uses). Events are handed
// to a `report` sink — typically the object returned by
// `WitslogBrowser.init(...)` from bindings/browser/witslog-browser.js,
// which ships them (batched, beacon/fetch) to `witslogBrowserIngest`
// (frameworks/express.js) or an equivalent server-side ingest handler.
//
//   // app/providers.tsx
//   import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
//   import WitslogBrowser from '@/lib/witslog-browser'; // bindings/browser/witslog-browser.js
//   import { attachWitslog } from 'witslog/frameworks/react-query';
//
//   const reporter = WitslogBrowser.init({ endpoint: '/__witslog', app: 'witsnote-client' });
//   const queryClient = new QueryClient();
//   attachWitslog(queryClient, { report: reporter, tags: ['witsnote'] });

/**
 * Normalize a caught error (which may be an `ApiError`-shaped object like
 * client/lib/api/errors.ts: `{code, message, status, details}`, a plain
 * Error, or anything thrown) into the witslog browser-ingest event shape
 * (see witslogBrowserIngest in frameworks/express.js for the accepted keys).
 */
function buildEvent(error, { tags, context, latencyMs }) {
  const err = error instanceof Error ? error : new Error(error != null ? String(error) : 'React Query error');
  // ApiError (client/lib/api/errors.ts) carries `.code`/`.status`/`.details`
  // directly on the error object — NOT under `.response.status` (that mismatch
  // is a real bug found in WitsNote's own hooks.ts:160 conflict-detection
  // branch). Reading `.code`/`.status` here matches the actual shape.
  const errorCode = typeof error?.code === 'string' ? error.code : undefined;
  const status = typeof error?.status === 'number' ? error.status : undefined;
  // witslogAxiosInterceptor (frameworks/axios.js) stamps these directly onto
  // the rejected error object — duck-typed read, same pattern as .code/.status.
  const correlationId = typeof error?.correlationId === 'string' ? error.correlationId : undefined;
  const axiosLatencyMs = typeof error?.latencyMs === 'number' ? error.latencyMs : undefined;

  return {
    message: err.message || 'React Query error',
    severity: 'error',
    exception: err.name,
    stacktrace: err.stack,
    error_code: errorCode,
    correlation_id: correlationId,
    tags,
    context: {
      ...context,
      http_status: status,
      timing: { latency_ms: axiosLatencyMs ?? latencyMs },
    },
  };
}

/** TanStack Query v5 state exposes submittedAt/errorUpdatedAt on mutations, dataUpdatedAt/errorUpdatedAt on queries. */
function computeLatencyMs(state) {
  const startedAt = typeof state?.submittedAt === 'number' ? state.submittedAt : undefined;
  const endedAt = typeof state?.errorUpdatedAt === 'number' ? state.errorUpdatedAt : undefined;
  return startedAt !== undefined && endedAt !== undefined && endedAt >= startedAt
    ? endedAt - startedAt
    : undefined;
}

function resolveEmit(report) {
  if (typeof report === 'function') return report;
  if (report && typeof report.enqueue === 'function') return (event) => report.enqueue(event);
  throw new TypeError(
    'attachWitslog requires opts.report: a function(event) or a {enqueue(event)} reporter ' +
      '(e.g. the object returned by WitslogBrowser.init(...) from bindings/browser/witslog-browser.js)'
  );
}

/**
 * Attach global mutation/query error capture to an existing QueryClient.
 * Returns a `detach()` function that unsubscribes both listeners.
 *
 * @param {{ getMutationCache: () => { subscribe: Function }, getQueryCache: () => { subscribe: Function } }} queryClient
 * @param {{
 *   report: Function | { enqueue: Function },
 *   tags?: string[],
 *   captureQueries?: boolean,
 * }} opts
 */
function attachWitslog(queryClient, opts = {}) {
  const { report, tags = [], captureQueries = true } = opts;
  const emit = resolveEmit(report);

  const mutationCache = queryClient.getMutationCache();
  const unsubMutation = mutationCache.subscribe((event) => {
    if (!event || event.type !== 'updated') return;
    const mutation = event.mutation;
    if (!mutation || mutation.state.status !== 'error') return;
    emit(
      buildEvent(mutation.state.error, {
        tags: [...tags, 'react-query', 'mutation'],
        latencyMs: computeLatencyMs(mutation.state),
        context: {
          mutationKey: mutation.options && mutation.options.mutationKey,
          variables: mutation.state.variables,
        },
      })
    );
  });

  let unsubQuery = () => {};
  if (captureQueries) {
    const queryCache = queryClient.getQueryCache();
    unsubQuery = queryCache.subscribe((event) => {
      if (!event || event.type !== 'updated') return;
      const query = event.query;
      if (!query || query.state.status !== 'error') return;
      emit(
        buildEvent(query.state.error, {
          tags: [...tags, 'react-query', 'query'],
          latencyMs: computeLatencyMs(query.state),
          context: { queryKey: query.queryKey },
        })
      );
    });
  }

  return function detach() {
    unsubMutation();
    unsubQuery();
  };
}

module.exports = { attachWitslog, __buildEvent: buildEvent };
