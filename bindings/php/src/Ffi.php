<?php

declare(strict_types=1);

namespace Witslog;

/** Locates + loads the native library via ext-ffi and validates the ABI version. */
final class Ffi
{
    /** ABI contract version this SDK is built for. Keep in sync with
     *  WITSLOG_ABI_VERSION in crates/witslog-ffi/src/lib.rs and bindings/CONTRACT.md. */
    public const ABI_VERSION = 1;

    private const CDEF = <<<'CDECL'
        int witslog_abi_version(void);
        int witslog_configure(const char *json);
        int witslog_init(const char *json);
        int64_t witslog_log(const char *json);
        int witslog_resolve(const char *event_id);
        int witslog_flush(void);
        int witslog_shutdown(void);
        CDECL;

    /** @return string[] candidate paths in locator order */
    public static function candidatePaths(): array
    {
        $paths = [];
        $env = getenv('WITSLOG_LIB');
        if ($env !== false && $env !== '') {
            $paths[] = $env;
        }
        $paths[] = __DIR__ . '/../_libs/' . self::platformDir() . '/' . self::libFilename();
        $paths[] = self::libFilename(); // OS default loader search
        return $paths;
    }

    public static function platformDir(): string
    {
        $plat = match (PHP_OS_FAMILY) {
            'Windows' => 'win32',
            'Darwin' => 'darwin',
            default => 'linux',
        };
        $arch = match (php_uname('m')) {
            'x86_64', 'AMD64' => 'x64',
            'arm64', 'aarch64' => 'arm64',
            default => php_uname('m'),
        };
        return "{$plat}-{$arch}";
    }

    public static function libFilename(): string
    {
        return match (PHP_OS_FAMILY) {
            'Windows' => 'witslog_ffi.dll',
            'Darwin' => 'libwitslog_ffi.dylib',
            default => 'libwitslog_ffi.so',
        };
    }

    /** Throw WitslogContractError unless the native ABI matches this SDK. */
    public static function checkAbi(int $actual, int $expected = self::ABI_VERSION): void
    {
        if ($actual !== $expected) {
            throw new WitslogContractError($expected, $actual);
        }
    }

    /**
     * Load the native library, returning the validated \FFI handle.
     *
     * @throws WitslogLibraryError when no candidate loads
     * @throws WitslogContractError on ABI mismatch
     */
    public static function load(): \FFI
    {
        $tried = self::candidatePaths();
        $handle = null;
        foreach ($tried as $candidate) {
            $isBare = $candidate === self::libFilename();
            if (!$isBare && !file_exists($candidate)) {
                continue;
            }
            try {
                $handle = \FFI::cdef(self::CDEF, $candidate);
                break;
            } catch (\FFI\Exception) {
                // try next candidate
            }
        }

        if ($handle === null) {
            throw new WitslogLibraryError($tried);
        }

        self::checkAbi((int) $handle->witslog_abi_version());
        return $handle;
    }
}
