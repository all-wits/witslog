'use strict';

// Express adapter. Mount witslog at startup, then register this AFTER your routes:
//
//   const witslog = require('witslog');
//   const { witslogErrorHandler } = require('witslog/frameworks/express');
//   witslog.init();
//   app.use(witslogErrorHandler('myapp'));   // last, after routes
//
// Logs any error passed to next(err), then forwards it to the next handler.

const witslog = require('../index');

function witslogErrorHandler(application = 'express') {
  return function (err, req, res, next) {
    try {
      witslog.exception(application, err, {
        context: { path: req && req.path, method: req && req.method },
      });
    } catch (_e) {
      /* never let logging mask the request error */
    }
    next(err);
  };
}

module.exports = { witslogErrorHandler };
