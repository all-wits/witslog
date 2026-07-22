'use strict';

// WebSocket close/disconnect capture — the network-tab-equivalent gap for
// long-lived connections (e.g. a Hocuspocus/Y.js collab provider), which
// today closes silently with nothing logged. Returns handler functions
// shaped to drop directly into HocuspocusProvider's constructor options
// (verified shape: onClose({event: CloseEvent}), onDisconnect({event:
// CloseEvent})) or any API that hands back a raw WebSocket CloseEvent.
//
//   // client/lib/collab/useBoardDoc.ts
//   import { witslogWebSocketWatch } from 'witslog-websocket'; // bindings/browser/witslog-websocket.js
//   const watch = witslogWebSocketWatch({ report: reporter, tags: ['witsnote'], context: { board: { boardId } } });
//   new HocuspocusProvider({ ..., onClose: watch.onClose, onDisconnect: watch.onDisconnect });

/** Pure — true for an abnormal close (anything but normal/going-away). */
function isAbnormalClose(code) {
  return code !== 1000 && code !== 1001;
}

/** Pure — builds the captured event from a CloseEvent-shaped object. */
function buildCloseEvent(closeEvent, { tags = [], context = {} } = {}) {
  const code = closeEvent && closeEvent.code;
  const reason = closeEvent && closeEvent.reason;
  const wasClean = closeEvent && closeEvent.wasClean;
  return {
    message: `WebSocket closed abnormally (code ${code})`,
    severity: 'error',
    error_code: `WS_CLOSE_${code}`,
    tags: ['network', 'websocket', ...tags],
    context: {
      ...context,
      ws: { code, reason, wasClean },
    },
  };
}

function resolveEmit(report) {
  if (typeof report === 'function') return report;
  if (report && typeof report.enqueue === 'function') return (event) => report.enqueue(event);
  throw new TypeError(
    'witslogWebSocketWatch requires opts.report: a function(event) or a {enqueue(event)} reporter'
  );
}

/**
 * @param {{
 *   report: Function | { enqueue: Function },
 *   tags?: string[],
 *   context?: Record<string, unknown>,
 * }} opts
 * @returns {{ onClose: Function, onDisconnect: Function }}
 */
function witslogWebSocketWatch(opts = {}) {
  const { tags, context } = opts;
  const emit = resolveEmit(opts.report);

  function handle({ event } = {}) {
    if (!event || !isAbnormalClose(event.code)) return;
    emit(buildCloseEvent(event, { tags, context }));
  }

  return { onClose: handle, onDisconnect: handle };
}

const WitslogWebSocket = { witslogWebSocketWatch, isAbnormalClose, __buildCloseEvent: buildCloseEvent };

if (typeof module !== 'undefined' && module.exports) {
  module.exports = WitslogWebSocket;
}
if (typeof window !== 'undefined') {
  window.WitslogWebSocket = WitslogWebSocket;
}
