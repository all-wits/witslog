'use strict';

// Pure marshalling — no native dependency, unit-testable in isolation.

const ALLOWED_FIELDS = new Set([
  'severity',
  'version',
  'environment',
  'category',
  'error_code',
  'exception',
  'stacktrace',
  'correlation_id',
  'parent_event_id',
  'context',
  'tags',
  'metadata',
]);

/**
 * Build the witslog_log JSON contract object. Throws TypeError/RangeError on
 * invalid input (FR-P6 error table).
 */
function buildPayload(application, message, fields = {}) {
  if (typeof application !== 'string') {
    throw new TypeError('application must be a string');
  }
  if (typeof message !== 'string') {
    throw new TypeError('message must be a string');
  }

  const payload = { application, message };
  for (const [key, value] of Object.entries(fields)) {
    if (value === undefined || value === null) continue;
    if (!ALLOWED_FIELDS.has(key)) {
      throw new RangeError(`unknown field: ${key}`);
    }
    if (key === 'tags' && !Array.isArray(value)) {
      throw new TypeError('tags must be an array of strings');
    }
    payload[key] = value;
  }
  return payload;
}

function encode(payload) {
  return JSON.stringify(payload);
}

module.exports = { buildPayload, encode, ALLOWED_FIELDS };
