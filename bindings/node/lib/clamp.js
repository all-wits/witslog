'use strict';

// Shared string-clamping helper — used by frameworks/express.js's browser
// ingest endpoint (untrusted input) and by fetch.js's error-response body
// snapshot (trusted, but still unbounded upstream output). Kept as a single
// pure function so both call sites stay in sync.

function clampString(value, maxLen) {
  if (typeof value !== 'string') return undefined;
  return value.length > maxLen ? value.slice(0, maxLen) : value;
}

/**
 * Recursively clamp an untrusted context object to bounded shape/size before
 * it is persisted (and later served verbatim to an MCP-connected LLM — see
 * the P10 gotcha on browser ingest). Keeps only string/number/boolean leaves
 * and plain-object/array nesting up to `maxDepth`; strings are clamped to
 * `maxStringLen`; object key count and array length are capped; the whole
 * result is capped to `maxTotalLen` serialized bytes (dropped entirely,
 * replaced with `{ _truncated: true }`, if still too large after clamping —
 * simpler and safer than a partial/ambiguous truncation).
 *
 * Used by both the browser-ingest endpoint (frameworks/express.js, fully
 * untrusted input) and the React Query adapter's captured mutation
 * variables/response (frameworks/react-query.js — trusted call site, but
 * still unbounded application data).
 */
function clampContext(ctx, opts = {}) {
  const { maxKeys = 20, maxDepth = 2, maxStringLen = 500, maxArrayLen = 10, maxTotalLen = 4000 } = opts;

  if (ctx === null || typeof ctx !== 'object' || Array.isArray(ctx)) return undefined;

  function walkValue(value, depth) {
    if (value === null || value === undefined) return undefined;
    if (typeof value === 'string') return clampString(value, maxStringLen);
    if (typeof value === 'number' || typeof value === 'boolean') return value;
    if (Array.isArray(value)) {
      if (depth >= maxDepth) return undefined;
      const out = [];
      for (const item of value.slice(0, maxArrayLen)) {
        const clamped = walkValue(item, depth + 1);
        if (clamped !== undefined) out.push(clamped);
      }
      return out;
    }
    if (typeof value === 'object') {
      if (depth >= maxDepth) return undefined;
      return walkObject(value, depth + 1);
    }
    return undefined; // functions, symbols, etc — never serialized
  }

  function walkObject(obj, depth) {
    const out = {};
    let count = 0;
    for (const [key, value] of Object.entries(obj)) {
      if (count >= maxKeys) break;
      if (typeof key !== 'string') continue;
      const clamped = walkValue(value, depth);
      if (clamped !== undefined) {
        out[key] = clamped;
        count += 1;
      }
    }
    return out;
  }

  const walked = walkObject(ctx, 0);
  if (JSON.stringify(walked).length > maxTotalLen) {
    return { _truncated: true };
  }
  return walked;
}

module.exports = { clampString, clampContext };
