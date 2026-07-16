<?php

declare(strict_types=1);

namespace Witslog;

/**
 * Framework-agnostic PHP SDK surface. Thin wrapper over the native library.
 *
 *   Witslog::init();
 *   Witslog::error('myapp', 'boom', ['context' => ['request_id' => 'r1'], 'tags' => ['db']]);
 *   try { ... } catch (\Throwable $e) { Witslog::exception('myapp', $e); }
 *   Witslog::shutdown();
 */
final class Witslog
{
    private static ?object $handle = null;
    private static bool $shutdownRegistered = false;

    /** Inject a fake FFI handle for testing. */
    public static function setHandleForTest(?object $handle): void
    {
        self::$handle = $handle;
    }

    private static function handle(): object
    {
        if (self::$handle === null) {
            self::$handle = Ffi::load();
        }
        return self::$handle;
    }

    /** @param array<string,mixed> $fields */
    public static function log(string $application, string $message, array $fields = []): int
    {
        $payload = Payload::build($application, $message, $fields);
        $rc = (int) self::handle()->witslog_log(Payload::encode($payload));
        if ($rc < 0) {
            throw new WitslogWriteError("witslog_log failed (rc={$rc}) for application={$application}");
        }
        return $rc;
    }

    /** @param array<string,mixed> $fields */
    public static function error(string $application, string $message, array $fields = []): int
    {
        return self::log($application, $message, ['severity' => 'error'] + $fields);
    }

    /** @param array<string,mixed> $fields */
    public static function warn(string $application, string $message, array $fields = []): int
    {
        return self::log($application, $message, ['severity' => 'warn'] + $fields);
    }

    /** @param array<string,mixed> $fields */
    public static function info(string $application, string $message, array $fields = []): int
    {
        return self::log($application, $message, ['severity' => 'info'] + $fields);
    }

    /** Log a Throwable with its message + trace captured. @param array<string,mixed> $fields */
    public static function exception(string $application, \Throwable $e, array $fields = []): int
    {
        $fields += [
            'severity' => 'error',
            'exception' => (new \ReflectionClass($e))->getShortName(),
            'stacktrace' => $e->getTraceAsString(),
        ];
        $message = $e->getMessage() !== '' ? $e->getMessage() : get_class($e);
        return self::log($application, $message, $fields);
    }

    /** Mount witslog for this process. @param array<string,mixed>|null $config */
    public static function init(?array $config = null): int
    {
        $json = $config === null ? null : Payload::encode($config);
        $rc = (int) self::handle()->witslog_init($json);
        if ($rc === -1) {
            throw new \InvalidArgumentException('witslog_init rejected the config JSON');
        }
        if ($rc === -2) {
            throw new \InvalidArgumentException('witslog_init rejected an invalid redaction pattern');
        }
        if (!self::$shutdownRegistered) {
            register_shutdown_function([self::class, 'shutdown']);
            self::$shutdownRegistered = true;
        }
        return $rc;
    }

    public static function flush(): int
    {
        return (int) self::handle()->witslog_flush();
    }

    public static function shutdown(): int
    {
        return (int) self::handle()->witslog_shutdown();
    }

    /** Route uncaught exceptions to witslog before PHP's default handler. */
    public static function installExceptionHandler(string $application = 'php'): void
    {
        $previous = set_exception_handler(null);
        set_exception_handler(function (\Throwable $e) use ($application, $previous): void {
            try {
                self::exception($application, $e, ['severity' => 'fatal']);
                self::flush();
            } catch (WitslogError) {
                // never mask the original crash
            }
            if ($previous !== null) {
                $previous($e);
            } else {
                throw $e;
            }
        });
    }
}
