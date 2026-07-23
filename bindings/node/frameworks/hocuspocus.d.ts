/** Minimal shape needed from a Hocuspocus provider — avoids a hard @hocuspocus/provider dependency. */
export interface HocuspocusProviderLike {
  on(event: string, fn: (...args: any[]) => void): unknown;
  off(event: string, fn: (...args: any[]) => void): unknown;
}

export interface WitslogReporterLike {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- accepts any event-shaped
  // object; Record<string, unknown> rejects plain interfaces without an index signature.
  enqueue(event: any): void;
  flush?(): void;
}

export interface WitslogHocuspocusOptions {
  /** Sink for captured events: a function(event), or a {enqueue(event), flush?()} reporter. */
  report: ((event: any) => void) | WitslogReporterLike;
  /** extra tags merged onto every captured event */
  tags?: string[];
  /** extra context merged onto every captured event (e.g. { board: { boardId } }) */
  context?: Record<string, unknown>;
}

/**
 * Attach abnormal-close/disconnect/authentication-failure capture to a
 * HocuspocusProvider. Always flushes the reporter immediately after each
 * captured event (connection-loss/auth-failure events are rare and urgent —
 * unlike high-volume adapters, this does not rely on the reporter's batch
 * window). Returns a `detach()` function that unsubscribes everything.
 */
export function attachWitslogHocuspocus(
  provider: HocuspocusProviderLike,
  opts: WitslogHocuspocusOptions
): () => void;
