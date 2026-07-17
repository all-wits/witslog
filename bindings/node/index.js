'use strict';

const { ABI_VERSION, loadLibrary } = require('./lib/ffi');
const { buildPayload, encode } = require('./lib/payload');
const errors = require('./lib/errors');

let _lib = null;
let _atexitRegistered = false;

function getLib() {
  if (_lib === null) {
    _lib = loadLibrary();
  }
  return _lib;
}

/** Log an event. Returns the DB rowid (or 0 when buffering). Throws on FFI error. */
function log(application, message, fields = {}) {
  const payload = buildPayload(application, message, fields);
  const rc = getLib().log(encode(payload));
  if (rc < 0) {
    throw new errors.WitslogWriteError(
      `witslog_log failed (rc=${rc}) for application=${application}`
    );
  }
  return rc;
}

function error(application, message, fields = {}) {
  return log(application, message, { severity: 'error', ...fields });
}

function warn(application, message, fields = {}) {
  return log(application, message, { severity: 'warn', ...fields });
}

function info(application, message, fields = {}) {
  return log(application, message, { severity: 'info', ...fields });
}

/** Log an Error object with its stack captured. */
function exception(application, err, fields = {}) {
  const out = { severity: 'error', ...fields };
  let message = fields.message;
  if (err instanceof Error) {
    out.exception = out.exception || err.name;
    out.stacktrace = out.stacktrace || err.stack || '';
    if (message === undefined) message = err.message || err.name;
  }
  if (message === undefined) message = 'exception';
  delete out.message;
  return log(application, message, out);
}

/**
 * Mount witslog for this process. `config` is the init/configure object (see CONTRACT.md).
 * Pass `createProject: true` (or a path string) to scaffold a `.witslog/` project directory
 * first — needed because `npm install` alone ships no CLI to run `witslog init` with.
 */
function init(config = null) {
  let rest = config;
  if (config && config.createProject) {
    const projectPath =
      typeof config.createProject === 'string' ? config.createProject : null;
    const bootstrapRc = getLib().bootstrapProject(projectPath);
    if (bootstrapRc !== 0) {
      throw new errors.WitslogWriteError(
        `witslog_bootstrap_project failed (rc=${bootstrapRc}) for path=${projectPath || process.cwd()}`
      );
    }
    const { createProject: _drop, ...withoutCreateProject } = config;
    rest = withoutCreateProject;
  }

  const json =
    rest === null || rest === undefined || Object.keys(rest).length === 0 ? null : encode(rest);
  const rc = getLib().init(json);
  if (rc === -1) throw new RangeError('witslog_init rejected the config JSON');
  if (rc === -2) throw new RangeError('witslog_init rejected an invalid redaction pattern');
  if (!_atexitRegistered) {
    process.on('exit', () => {
      try {
        shutdown();
      } catch (_e) {
        /* never throw during exit */
      }
    });
    _atexitRegistered = true;
  }
  return rc;
}

function flush() {
  return getLib().flush();
}

function shutdown() {
  return getLib().shutdown();
}

/** Route uncaught exceptions / rejections to witslog, then re-throw / exit. */
function installUncaughtHandler(application = 'node') {
  process.on('uncaughtException', (err) => {
    try {
      log(application, err.message || err.name || 'uncaughtException', {
        severity: 'fatal',
        exception: err.name,
        stacktrace: err.stack || '',
      });
      flush();
    } catch (_e) {
      /* never mask the original crash */
    }
    throw err;
  });

  process.on('unhandledRejection', (reason) => {
    try {
      const err = reason instanceof Error ? reason : new Error(String(reason));
      log(application, err.message, {
        severity: 'error',
        exception: err.name,
        stacktrace: err.stack || '',
      });
    } catch (_e) {
      /* swallow */
    }
  });
}

// Test hook: inject a fake lib so error paths can be exercised without the dll.
function __setLibForTest(fake) {
  _lib = fake;
}

module.exports = {
  ABI_VERSION,
  log,
  error,
  warn,
  info,
  exception,
  init,
  flush,
  shutdown,
  installUncaughtHandler,
  buildPayload,
  ...errors,
  __setLibForTest,
};
