<?php

declare(strict_types=1);

// PHP SDK e2e smoke: mount, log (with context+tags), exception, flush.
// Run with FFI enabled, from inside a `.witslog/` project dir, WITSLOG_LIB set:
//   php -d extension=ffi -d ffi.enable=1 php_smoke.php <marker>
// Requires the composer autoloader path via WITSLOG_AUTOLOAD.

$autoload = getenv('WITSLOG_AUTOLOAD') ?: (__DIR__ . '/../php/vendor/autoload.php');
require $autoload;

use Witslog\Witslog;

$marker = $argv[1] ?? 'PHPSMOKE';
$argvMode = $argv[2] ?? 'argv-on';
echo 'abi ' . \Witslog\Ffi::ABI_VERSION . "\n";

if ($argvMode === 'argv-off') {
    // Regression lock: enrich.argv=false must fully suppress argv capture,
    // closing the CLI-arg-secret exposure documented in CONTRACT.md.
    Witslog::init(['enrich' => ['argv' => false]]);
} else {
    Witslog::init();
}

$rowid = Witslog::error('php-e2e', "php sdk event {$marker}", [
    'context' => ['request_id' => "{$marker}-req", 'pid' => 1],
    'tags' => [$marker, 'php', "TAG{$marker}"],
    'metadata' => ['lang' => 'php'],
]);
echo "rowid {$rowid}\n";
if ($rowid < 0) {
    fwrite(STDERR, "log returned an error\n");
    exit(1);
}

try {
    throw new \RuntimeException("boom {$marker}");
} catch (\RuntimeException $e) {
    Witslog::exception('php-e2e', $e, ['tags' => [$marker]]);
}

Witslog::shutdown();
echo "PHP_SMOKE_OK {$marker}\n";
