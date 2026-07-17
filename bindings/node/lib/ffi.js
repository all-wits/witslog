'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');

const { WitslogLibraryError, WitslogContractError } = require('./errors');

// ABI contract version this SDK is built for. Keep in sync with
// WITSLOG_ABI_VERSION in crates/witslog-ffi/src/lib.rs and bindings/CONTRACT.md.
const ABI_VERSION = 1;

function platformDir() {
  const plat = process.platform; // 'win32' | 'linux' | 'darwin'
  const arch = { x64: 'x64', arm64: 'arm64', ia32: 'x86' }[process.arch] || process.arch;
  return `${plat}-${arch}`;
}

function libFilename() {
  if (process.platform === 'win32') return 'witslog_ffi.dll';
  if (process.platform === 'darwin') return 'libwitslog_ffi.dylib';
  return 'libwitslog_ffi.so';
}

function candidatePaths() {
  const paths = [];
  if (process.env.WITSLOG_LIB) paths.push(process.env.WITSLOG_LIB);
  paths.push(path.join(__dirname, '..', '_libs', platformDir(), libFilename()));
  paths.push(libFilename()); // OS default loader search
  return paths;
}

/** Assert the native ABI version matches this SDK; throws WitslogContractError otherwise. */
function checkAbi(actual, expected = ABI_VERSION) {
  if (actual !== expected) {
    throw new WitslogContractError(expected, actual);
  }
}

/**
 * Locate and load the native library via koffi. Returns a typed wrapper.
 * Throws WitslogLibraryError if nothing loads, WitslogContractError on mismatch.
 */
function loadLibrary(koffi = require('koffi')) {
  const tried = candidatePaths();
  let lib = null;
  let loadedFrom = null;

  for (const candidate of tried) {
    // For explicit/bundled paths, skip non-existent files so the error lists them all.
    const isBare = candidate === libFilename();
    if (!isBare && !fs.existsSync(candidate)) continue;
    try {
      lib = koffi.load(candidate);
      loadedFrom = candidate;
      break;
    } catch (_e) {
      // try next candidate
    }
  }

  if (!lib) {
    throw new WitslogLibraryError(tried);
  }

  const fns = {
    abi_version: lib.func('int witslog_abi_version()'),
    configure: lib.func('int witslog_configure(const char*)'),
    init: lib.func('int witslog_init(const char*)'),
    log: lib.func('int64 witslog_log(const char*)'),
    resolve: lib.func('int witslog_resolve(const char*)'),
    bootstrap_project: lib.func('int witslog_bootstrap_project(const char*)'),
    flush: lib.func('int witslog_flush()'),
    shutdown: lib.func('int witslog_shutdown()'),
  };

  checkAbi(Number(fns.abi_version()));

  return {
    loadedFrom,
    abiVersion: () => Number(fns.abi_version()),
    configure: (json) => Number(fns.configure(json)),
    init: (json) => Number(fns.init(json)),
    log: (json) => Number(fns.log(json)),
    resolve: (id) => Number(fns.resolve(id)),
    bootstrapProject: (path) => Number(fns.bootstrap_project(path)),
    flush: () => Number(fns.flush()),
    shutdown: () => Number(fns.shutdown()),
  };
}

module.exports = { ABI_VERSION, loadLibrary, checkAbi, candidatePaths, libFilename, platformDir };
