'use strict';

class WitslogError extends Error {
  constructor(message) {
    super(message);
    this.name = 'WitslogError';
  }
}

class WitslogLibraryError extends WitslogError {
  constructor(searchedPaths) {
    const joined = searchedPaths.length ? '\n  ' + searchedPaths.join('\n  ') : '(none)';
    super(
      'could not locate the native witslog library. Set the WITSLOG_LIB ' +
        'environment variable to its path, or bundle it under _libs/<platform>/. ' +
        `Searched:${joined}`
    );
    this.name = 'WitslogLibraryError';
    this.searchedPaths = searchedPaths.slice();
  }
}

class WitslogContractError extends WitslogError {
  constructor(expected, actual) {
    super(
      `witslog contract mismatch: SDK expects ABI version ${expected}, ` +
        `native library reports ${actual}. Upgrade the SDK or the native library.`
    );
    this.name = 'WitslogContractError';
    this.expected = expected;
    this.actual = actual;
  }
}

class WitslogWriteError extends WitslogError {
  constructor(message) {
    super(message);
    this.name = 'WitslogWriteError';
  }
}

class WitslogCliNotFoundError extends WitslogError {
  constructor(searchedPaths) {
    const joined = searchedPaths.length ? '\n  ' + searchedPaths.join('\n  ') : '(none)';
    super(
      'could not locate the witslog CLI binary. Set the WITSLOG_CLI ' +
        'environment variable to its path, or bundle it under _bin/<platform>/. ' +
        `Searched:${joined}`
    );
    this.name = 'WitslogCliNotFoundError';
    this.searchedPaths = searchedPaths.slice();
  }
}

module.exports = {
  WitslogError,
  WitslogLibraryError,
  WitslogContractError,
  WitslogWriteError,
  WitslogCliNotFoundError,
};
