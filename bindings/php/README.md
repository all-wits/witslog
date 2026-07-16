# witslog (PHP / Laravel SDK)

Framework-agnostic PHP SDK over the native witslog library using PHP's built-in `ext-ffi` —
no third-party runtime deps. See [../CONTRACT.md](../CONTRACT.md) for the ABI.

Enable FFI (default `php.ini` ships it off): set `extension=ffi` and `ffi.enable=1`, or pass
`php -d extension=ffi -d ffi.enable=1`.

```php
use Witslog\Witslog;

Witslog::init();                       // mount once (register_shutdown_function flushes)
Witslog::error('myapp', 'db timeout', ['context' => ['request_id' => 'r1'], 'tags' => ['db']]);

try {
    risky();
} catch (\Throwable $e) {
    Witslog::exception('myapp', $e);   // captures getTraceAsString()
}
```

Locate the native library via `WITSLOG_LIB` (dev/CI) or bundle it under `_libs/<platform>/`.
Run from a directory inside a `.witslog/` project (or `witslog init` one first).

> **Security:** `argv` enrichment defaults on and captures the full command line. If your app
> may receive secrets as bare CLI args, call `Witslog::init(['enrich' => ['argv' => false]])` —
> see [../CONTRACT.md](../CONTRACT.md#security-note-argv-enrichment-vs-secrets).

## Laravel

Auto-discovered (`extra.laravel.providers`). To capture rendered exceptions, in Laravel 11+
`bootstrap/app.php`:

```php
->withExceptions(function (Illuminate\Foundation\Configuration\Exceptions $exceptions) {
    $exceptions->reportable(fn (\Throwable $e) =>
        \Witslog\Witslog::exception(config('app.name', 'laravel'), $e));
})
```

For Laravel 10, call `Witslog::exception(...)` from `App\Exceptions\Handler::report()`.

## Test

```
composer install --ignore-platform-req=ext-ffi
php -d extension=ffi -d ffi.enable=1 vendor/bin/phpunit
```
