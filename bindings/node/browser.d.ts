export interface WitslogBrowserEvent {
  message: string;
  severity?: 'error' | 'warn';
  exception?: string;
  stacktrace?: string;
  error_code?: string;
  correlation_id?: string;
  tags?: string[];
  context?: Record<string, unknown>;
}

export interface WitslogBrowserConfig {
  endpoint: string;
  app?: string;
  sampleRate?: number;
  /**
   * Also capture `console.error` (severity `error`) / `console.warn`
   * (severity `warn`) calls — tagged `['console']` — and capture-phase
   * resource-load failures (`<img>`/`<script>`/`<link>` 404s etc, tagged
   * `['resource']`), which don't throw and are otherwise invisible to
   * `window.onerror`/`unhandledrejection`. Default `false` — opt-in because
   * it patches a global and can be noisy.
   */
  captureConsole?: boolean;
}

export interface WitslogBrowserReporter {
  flush: () => void;
  enqueue: (event: WitslogBrowserEvent) => void;
}

export interface WitslogBrowserStatic {
  init(config: WitslogBrowserConfig): WitslogBrowserReporter;
  buildBatch(events: WitslogBrowserEvent[], meta: { app: string }): unknown;
  makeErrorEvent(message: unknown, opts?: Partial<WitslogBrowserEvent>): WitslogBrowserEvent;
}

declare const WitslogBrowser: WitslogBrowserStatic;
export default WitslogBrowser;
