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
  enqueue(event: Record<string, unknown>): void;
}

export interface WitslogAxiosOptions {
  /** Sink for direct-capture events (requests with `witslogDirectCapture: true`). */
  report?: ((event: Record<string, unknown>) => void) | WitslogReporterLike;
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
