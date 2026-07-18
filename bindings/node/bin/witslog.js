#!/usr/bin/env node
'use strict';

const { spawnSync } = require('child_process');

const { resolveCliPath, candidateCliPaths } = require('../lib/cli-locator');
const { WitslogCliNotFoundError } = require('../lib/errors');

const cliPath = resolveCliPath();
const result = spawnSync(cliPath, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  if (result.error.code === 'ENOENT') {
    console.error(new WitslogCliNotFoundError(candidateCliPaths()).message);
  } else {
    console.error(result.error.message);
  }
  process.exit(1);
}

process.exit(result.status === null ? 1 : result.status);
