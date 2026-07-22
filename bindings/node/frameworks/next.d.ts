/** Mount witslog once for this process. Call from instrumentation.ts's `register()`. */
export function register(application?: string, config?: Record<string, unknown> | null): void;

export interface NextRequestErrorRequest {
  path?: string;
  method?: string;
  headers?: Record<string, unknown>;
}

export interface NextRequestErrorContext {
  routerKind?: string;
  routePath?: string;
  routeType?: string;
  renderSource?: string;
  revalidateReason?: string;
}

/** Next.js's official server-error hook (Next.js 15+). Re-export from instrumentation.ts. */
export function onRequestError(
  err: unknown,
  request: NextRequestErrorRequest,
  context?: NextRequestErrorContext
): void;

export interface WithWitslogOptions {
  application?: string;
  tags?: string[];
}

/** Higher-order wrapper for a single route handler (Next < 15, or explicit per-route capture). */
export function withWitslog<
  Handler extends (request: any, ctx: any) => Promise<any>
>(handler: Handler, opts?: WithWitslogOptions): Handler;

export interface WitslogNextIngestOptions {
  application?: string;
  maxBatch?: number;
  maxBytes?: number;
  allowedOrigins?: string[];
  rateLimit?: { windowMs: number; max: number };
  force?: boolean;
}

/** Next.js Route Handler for the browser-ingest endpoint (P10d). */
export function witslogNextIngest(
  options?: WitslogNextIngestOptions
): (request: Request) => Promise<Response>;
