'use strict';

// Hocuspocus/Yjs collab-provider adapter — captures abnormal WebSocket
// closes/disconnects and authentication failures with zero per-app
// boilerplate. Mirrors frameworks/react-query.js's shape: duck-typed
// against the target's public event API (HocuspocusProvider extends
// EventEmitter, exposing on(event, fn)/off(event, fn)), so this has no
// hard @hocuspocus/provider dependency.
//
//   import { HocuspocusProvider } from '@hocuspocus/provider';
//   import { attachWitslogHocuspocus } from 'witslog/frameworks/hocuspocus';
//
//   const hp = new HocuspocusProvider({ url, name, document, token });
//   const detach = attachWitslogHocuspocus(hp, { report: reporter, tags: ['witsnote'] });
//   // later: detach(); hp.destroy();
//
// Unlike react-query.js/axios.js (high-volume, fine to batch), connection
// loss/auth failure events here are rare and urgent — this adapter always
// flushes the reporter immediately after enqueueing, rather than relying on
// the caller's batch window (pagehide/visibilitychange/periodic flush).

function resolveEmit(report) {
  if (typeof report === 'function') return report;
  if (report && typeof report.enqueue === 'function') return (event) => report.enqueue(event);
  throw new TypeError(
    'attachWitslogHocuspocus requires opts.report: a function(event) or a {enqueue(event)} reporter ' +
      '(e.g. the object returned by WitslogBrowser.init(...) from bindings/browser/witslog-browser.js)'
  );
}

function resolveFlush(report) {
  if (report && typeof report.flush === 'function') return () => report.flush();
  return () => {};
}

/**
 * Pure — true for an abnormal close (anything but normal/going-away).
 * A clean close (wasClean: true) is never abnormal even if the browser
 * reports code 1005 ("No Status Rcvd") — that code is synthesized locally
 * whenever the server closes without an explicit status, which happens on
 * routine disconnects (provider.destroy(), tab nav, HMR reload).
 */
function isAbnormalClose(code, wasClean) {
  if (wasClean) return false;
  return code !== 1000 && code !== 1001;
}

function buildCloseEvent(closeEvent, tags, context) {
  const { code, reason, wasClean } = closeEvent;
  return {
    message: `WebSocket closed abnormally (code ${code})`,
    severity: 'error',
    error_code: `WS_CLOSE_${code}`,
    tags: ['network', 'websocket', ...tags],
    context: { ...context, ws: { code, reason, wasClean } },
  };
}

function buildAuthFailedEvent(reason, tags, context) {
  return {
    message: `Collab authentication failed${reason ? `: ${reason}` : ''}`,
    severity: 'error',
    error_code: 'COLLAB_AUTH_FAILED',
    tags: ['network', 'websocket', ...tags],
    context,
  };
}

/**
 * Attach abnormal-close/disconnect/authentication-failure capture to a
 * HocuspocusProvider (or any EventEmitter-shaped target exposing
 * on(event, fn)/off(event, fn) and emitting 'close'/'disconnect' with
 * `{event: {code, reason, wasClean}}` and 'authenticationFailed' with
 * `{reason}`, per @hocuspocus/provider's public API).
 *
 * @param {{ on: Function, off: Function }} provider
 * @param {{
 *   report: Function | { enqueue: Function, flush?: Function },
 *   tags?: string[],
 *   context?: Record<string, unknown>,
 * }} opts
 * @returns {() => void} detach — unsubscribes everything this attached
 */
function attachWitslogHocuspocus(provider, opts = {}) {
  const { tags = [], context = {} } = opts;
  const emit = resolveEmit(opts.report);
  const flush = resolveFlush(opts.report);

  function onClose({ event } = {}) {
    if (!event || !isAbnormalClose(event.code, event.wasClean)) return;
    emit(buildCloseEvent(event, tags, context));
    flush();
  }

  function onAuthenticationFailed({ reason } = {}) {
    emit(buildAuthFailedEvent(reason, tags, context));
    flush();
  }

  provider.on('close', onClose);
  provider.on('disconnect', onClose);
  provider.on('authenticationFailed', onAuthenticationFailed);

  return function detach() {
    provider.off('close', onClose);
    provider.off('disconnect', onClose);
    provider.off('authenticationFailed', onAuthenticationFailed);
  };
}

module.exports = { attachWitslogHocuspocus, __isAbnormalClose: isAbnormalClose };
