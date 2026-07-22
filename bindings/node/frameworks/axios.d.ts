/** Minimal shape needed from an axios instance — avoids a hard axios dependency. */
export interface AxiosInstanceLike {
  interceptors: {
    request: { use(onFulfilled: (config: any) => any): number; eject(id: number): void };
    response: {
      use(onFulfilled: (response: any) => any, onRejected: (error: any) => any): number;
      eject(id: number): void;
    };
  };
}

export interface WitslogReporterLike {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- accepts any event-shaped
  // object (e.g. bindings/browser/witslog-browser.js's WitslogEvent, or a TS port's stricter
  // interface); Record<string, unknown> rejects plain interfaces without an index signature.
  enqueue(event: any): void;
}

export interface WitslogAxiosOptions {
  /** Sink for direct-capture events (requests with `witslogDirectCapture: true`). */
  report?: ((event: any) => void) | WitslogReporterLike;
  /** extra tags merged onto directly-captured events */
  tags?: string[];
  /** header name used to propagate the correlation id (default 'x-request-id') */
  correlationHeader?: string;
}

/**
 * Attach a request/response interceptor pair that mints/reuses a correlation
 * id per request and stamps `correlationId`/`latencyMs` onto the rejected
 * error object. Returns a `detach()` function that ejects both interceptors.
 */
export function witslogAxiosInterceptor(
  axiosInstance: AxiosInstanceLike,
  opts?: WitslogAxiosOptions
): () => void;
