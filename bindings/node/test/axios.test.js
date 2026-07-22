'use strict';

// Unit tests for the axios interceptor (bindings/node/frameworks/axios.js).
// A fake axios instance (duck-typed: interceptors.request/response.use/eject)
// stands in for a real axios instance — matches the adapter's no-hard-dep design.

const { test } = require('node:test');
const assert = require('node:assert');

const { witslogAxiosInterceptor } = require('../frameworks/axios');

function fakeAxios() {
  let onFulfilledReq;
  let onFulfilledRes;
  let onRejectedRes;
  return {
    interceptors: {
      request: {
        use: (fn) => {
          onFulfilledReq = fn;
          return 1;
        },
        eject: () => {},
      },
      response: {
        use: (fulfilled, rejected) => {
          onFulfilledRes = fulfilled;
          onRejectedRes = rejected;
          return 2;
        },
        eject: () => {},
      },
    },
    __runRequest: (config) => onFulfilledReq(config),
    __runResponseSuccess: (response) => onFulfilledRes(response),
    __runResponseError: (error) => onRejectedRes(error).catch(() => {}),
  };
}

test('mints a correlation id header when none present', () => {
  const ax = fakeAxios();
  witslogAxiosInterceptor(ax, {});
  const config = ax.__runRequest({ headers: {} });
  assert.ok(config.headers['x-request-id']);
  assert.strictEqual(config.__witslogCorrelationId, config.headers['x-request-id']);
});

test('reuses an existing correlation id header instead of minting a new one', () => {
  const ax = fakeAxios();
  witslogAxiosInterceptor(ax, {});
  const config = ax.__runRequest({ headers: { 'x-request-id': 'existing-id' } });
  assert.strictEqual(config.headers['x-request-id'], 'existing-id');
  assert.strictEqual(config.__witslogCorrelationId, 'existing-id');
});

test('stamps correlationId/latencyMs onto a rejected error, does not double-log by default', async () => {
  const ax = fakeAxios();
  const events = [];
  witslogAxiosInterceptor(ax, { report: (e) => events.push(e) });

  const config = ax.__runRequest({ headers: {} });
  const error = { config, message: 'Network Error' };
  await ax.__runResponseError(error);

  assert.strictEqual(error.correlationId, config.__witslogCorrelationId);
  assert.strictEqual(typeof error.latencyMs, 'number');
  assert.strictEqual(events.length, 0);
});

test('witslogDirectCapture:true on the request config directly logs the error', async () => {
  const ax = fakeAxios();
  const events = [];
  witslogAxiosInterceptor(ax, { report: (e) => events.push(e), tags: ['witsnote'] });

  const config = ax.__runRequest({ headers: {}, witslogDirectCapture: true, method: 'get', url: '/x' });
  const error = { config, message: 'boom', name: 'Error' };
  await ax.__runResponseError(error);

  assert.strictEqual(events.length, 1);
  assert.strictEqual(events[0].correlation_id, config.__witslogCorrelationId);
  assert.deepStrictEqual(events[0].tags, ['witsnote', 'network', 'axios']);
});

test('stamps correlationId/latencyMs onto a successful response too', () => {
  const ax = fakeAxios();
  witslogAxiosInterceptor(ax, {});
  const config = ax.__runRequest({ headers: {} });
  const response = { config, status: 200 };
  ax.__runResponseSuccess(response);
  assert.strictEqual(response.correlationId, config.__witslogCorrelationId);
  assert.strictEqual(typeof response.latencyMs, 'number');
});

test('detach() ejects both interceptors', () => {
  const ax = fakeAxios();
  let requestEjected = false;
  let responseEjected = false;
  ax.interceptors.request.eject = () => {
    requestEjected = true;
  };
  ax.interceptors.response.eject = () => {
    responseEjected = true;
  };
  const detach = witslogAxiosInterceptor(ax, {});
  detach();
  assert.ok(requestEjected);
  assert.ok(responseEjected);
});
