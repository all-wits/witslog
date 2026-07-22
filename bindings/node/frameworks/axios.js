'use strict';

// Axios interceptor — correlation-id propagation + latency capture for the
// shared `apiClient` axios instance. Mirrors fetch.js's shape (correlation
// header mint/reuse, timing) but does NOT log every rejected request itself
// — attachWitslog (frameworks/react-query.js) already captures every
// React-Query-managed mutation/query failure, so logging here too would
// double-log. Instead this interceptor:
//   1. mints/reuses a correlation id per request, attaches it as a header
//      (propagates browser -> proxy -> upstream, since witslogFetch already
//      reads an inbound x-request-id as its own correlation-id fallback);
//   2. stamps `correlationId`/`latencyMs` onto the rejected error object so
//      downstream consumers (ApiError, react-query.js's buildEvent) can read
//      them without a second network-layer log;
//   3. optionally directly captures calls that bypass React Query entirely,
//      via a per-request `witslogDirectCapture: true` config flag (used for
//      one-off calls like a collab-ticket fetch).
//
// Browser-safe: no Node built-ins beyond crypto.randomUUID (available in
// both Node >=19 and every modern browser).
//
//   // app/providers.tsx
//   import { witslogAxiosInterceptor } from 'witslog/frameworks/axios';
//   witslogAxiosInterceptor(apiClient, { report: reporter, tags: ['witsnote'] });
//
//   // client/lib/collab/ticket.ts — direct capture, bypasses React Query
//   apiClient.get('/collab/ticket', { witslogDirectCapture: true });

const DEFAULT_CORRELATION_HEADER = 'x-request-id';

function randomUUID() {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  // eslint-disable-next-line global-require
  return require('node:crypto').randomUUID();
}

function resolveEmit(report) {
  if (typeof report === 'function') return report;
  if (report && typeof report.enqueue === 'function') return (event) => report.enqueue(event);
  return null; // direct capture is opt-in per request; no report means no-op
}

/**
 * Attach a request/response interceptor pair to an axios instance.
 *
 * @param {{ interceptors: { request: { use: Function }, response: { use: Function } } }} axiosInstance
 * @param {{
 *   report?: Function | { enqueue: Function },
 *   tags?: string[],
 *   correlationHeader?: string,
 * }} opts
 */
function witslogAxiosInterceptor(axiosInstance, opts = {}) {
  const { report, tags = [], correlationHeader = DEFAULT_CORRELATION_HEADER } = opts;
  const emit = resolveEmit(report);

  const reqId = axiosInstance.interceptors.request.use((config) => {
    const existing = config.headers && config.headers[correlationHeader];
    const correlationId = existing || randomUUID();
    config.headers = { ...(config.headers || {}), [correlationHeader]: correlationId };
    config.__witslogCorrelationId = correlationId;
    config.__witslogStartedAt = Date.now();
    return config;
  });

  const resId = axiosInstance.interceptors.response.use(
    (response) => {
      annotate(response.config, response);
      return response;
    },
    (error) => {
      const config = error && error.config;
      annotate(config, error);
      if (config && config.witslogDirectCapture && emit) {
        emit({
          message: (error && error.message) || 'axios request failed',
          severity: 'error',
          exception: error && error.name,
          stacktrace: error && error.stack,
          error_code: error && error.code,
          correlation_id: config.__witslogCorrelationId,
          tags: [...tags, 'network', 'axios'],
          context: {
            correlationId: config.__witslogCorrelationId,
            latencyMs: config.__witslogLatencyMs,
            method: config.method,
            url: config.url,
          },
        });
      }
      return Promise.reject(error);
    }
  );

  return function detach() {
    axiosInstance.interceptors.request.eject(reqId);
    axiosInstance.interceptors.response.eject(resId);
  };
}

/** Stamp correlationId/latencyMs onto the target (response or error) from its request config. */
function annotate(config, target) {
  if (!config || !target) return;
  const latencyMs =
    typeof config.__witslogStartedAt === 'number' ? Date.now() - config.__witslogStartedAt : undefined;
  target.correlationId = config.__witslogCorrelationId;
  target.latencyMs = latencyMs;
  config.__witslogLatencyMs = latencyMs;
}

module.exports = { witslogAxiosInterceptor };
