export interface WitslogFetchOptions {
  /** witslog application name (default: 'fetch') */
  application?: string;
  /** extra tags merged onto every captured event */
  tags?: string[];
  /** extra context merged onto every captured event */
  context?: Record<string, unknown>;
  /** reuse an existing id (e.g. an inbound x-request-id) instead of minting a new one */
  correlationId?: string;
  /** header name to read/set (default 'x-request-id') */
  correlationHeader?: string;
  /** byte cap for the captured error-response body (default 4096) */
  maxBodySnapshot?: number;
}

/**
 * Fetch with automatic witslog capture. Behaves exactly like `fetch()` to
 * the caller — returns the real Response / throws the real error; the
 * response body is untouched (peeked via `.clone()`).
 */
export function witslogFetch(
  input: RequestInfo | URL,
  init?: RequestInit,
  opts?: WitslogFetchOptions
): Promise<Response>;
