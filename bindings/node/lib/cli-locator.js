'use strict';

const fs = require('fs');
const path = require('path');

const { platformDir } = require('./ffi');

function binFilename() {
  return process.platform === 'win32' ? 'witslog.exe' : 'witslog';
}

/** Candidate CLI paths in locator order (mirrors ffi.js's candidatePaths). */
function candidateCliPaths() {
  const paths = [];
  if (process.env.WITSLOG_CLI) paths.push(process.env.WITSLOG_CLI);
  paths.push(path.join(__dirname, '..', '_bin', platformDir(), binFilename()));
  paths.push(binFilename()); // bare name -> OS PATH search, so a separately-installed CLI still works
  return paths;
}

/**
 * Resolve the path to spawn for the witslog CLI. Never throws: the last
 * candidate is always the bare filename, deferring existence/PATH resolution
 * to the OS at spawn time (mirrors how `WitslogCliNotFoundError` is instead
 * raised from the spawn-time ENOENT in bin/witslog.js, not here).
 */
function resolveCliPath() {
  const tried = candidateCliPaths();
  for (const candidate of tried) {
    const isBare = candidate === binFilename();
    if (isBare) return candidate;
    if (fs.existsSync(candidate)) return candidate;
  }
  return binFilename();
}

module.exports = { binFilename, candidateCliPaths, resolveCliPath };
