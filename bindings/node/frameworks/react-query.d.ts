/** Minimal shape needed from a TanStack QueryClient — avoids a hard @tanstack/react-query dependency. */
export interface QueryClientLike {
  getMutationCache(): { subscribe(listener: (event: any) => void): () => void };
  getQueryCache(): { subscribe(listener: (event: any) => void): () => void };
}

/**
 * The event shape attachWitslog's internal buildEvent() actually produces
 * (react-query.js) — `message` is always set; everything else mirrors the
 * witslogBrowserIngest/witslogNextIngest accepted fields (see
 * bindings/CONTRACT.md). Matches bindings/browser/witslog-browser.js's
 * WitslogEvent shape so `WitslogBrowser.init(...)`'s returned reporter is
 * directly assignable as `report` without a cast.
 */
export interface CapturedEvent {
  message: string;
  severity?: string;
  exception?: string;
  stacktrace?: string;
  error_code?: string;
  tags?: string[];
  context?: Record<string, unknown>;
}

export interface WitslogReporterLike {
  enqueue(event: CapturedEvent): void;
}

export interface AttachWitslogOptions {
  /** Sink for captured events: a function(event), or a {enqueue(event)} reporter
   *  (e.g. the object returned by WitslogBrowser.init(...)). */
  report: ((event: CapturedEvent) => void) | WitslogReporterLike;
  /** extra tags merged onto every captured event */
  tags?: string[];
  /** set false to only capture mutations, not queries (default true) */
  captureQueries?: boolean;
}

/**
 * Attach global mutation/query error capture to an existing QueryClient.
 * Returns a `detach()` function that unsubscribes both listeners.
 */
export function attachWitslog(queryClient: QueryClientLike, opts: AttachWitslogOptions): () => void;
