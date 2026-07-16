export interface WitslogFields {
  severity?: 'trace' | 'debug' | 'info' | 'warn' | 'error' | 'critical' | 'fatal';
  version?: string;
  environment?: string;
  category?: string;
  error_code?: string;
  exception?: string;
  stacktrace?: string;
  correlation_id?: string;
  parent_event_id?: string;
  context?: Record<string, unknown>;
  tags?: string[];
  metadata?: Record<string, unknown>;
  message?: string;
}

export interface WitslogConfig {
  enrich?: Record<string, unknown>;
  redact?: { custom_patterns?: string[] };
  buffer?: {
    enabled?: boolean;
    batch_size?: number;
    flush_interval_ms?: number;
    queue_capacity?: number;
  };
}

export const ABI_VERSION: number;

export function log(application: string, message: string, fields?: WitslogFields): number;
export function error(application: string, message: string, fields?: WitslogFields): number;
export function warn(application: string, message: string, fields?: WitslogFields): number;
export function info(application: string, message: string, fields?: WitslogFields): number;
export function exception(application: string, err: Error, fields?: WitslogFields): number;

export function init(config?: WitslogConfig | null): number;
export function flush(): number;
export function shutdown(): number;
export function installUncaughtHandler(application?: string): void;

export function buildPayload(
  application: string,
  message: string,
  fields?: WitslogFields
): Record<string, unknown>;

export class WitslogError extends Error {}
export class WitslogLibraryError extends WitslogError {
  searchedPaths: string[];
}
export class WitslogContractError extends WitslogError {
  expected: number;
  actual: number;
}
export class WitslogWriteError extends WitslogError {}
