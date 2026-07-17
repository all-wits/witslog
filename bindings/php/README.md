<div align="center">

# 🪵 witslog (PHP / Laravel SDK)

[![Packagist](https://img.shields.io/packagist/v/witslog/witslog?logo=packagist)](https://packagist.org/packages/witslog/witslog)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../../LICENSE)
[![PHP](https://img.shields.io/packagist/php-v/witslog/witslog?logo=php)](https://packagist.org/packages/witslog/witslog)

**Framework-agnostic PHP SDK for [witslog](../../README.md) structured error logging.**

</div>

---

Thin wrapper over the native `witslog-ffi` C ABI using PHP's built-in **`ext-ffi`** — no
third-party runtime dependency. See [../CONTRACT.md](../CONTRACT.md) for the full SDK↔native
ABI.

## 📦 Install

```bash
composer require witslog/witslog
```

`ext-ffi` ships with PHP but is **off by default**. Enable it in `php.ini`:

```ini
extension=ffi
ffi.enable=1
```

Or per-invocation:

```bash
php -d extension=ffi -d ffi.enable=1 your-script.php
```

## 🚀 Quick Start

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

Run from a directory inside a `.witslog/` project (or `witslog init` one first) so events
land in that project's DB. This SDK doesn't yet wrap the native `witslog_bootstrap_project`
export (see [../CONTRACT.md](../CONTRACT.md#witslog_bootstrap_project-no-json--plain-path-string-or-null))
the way the [Node SDK's](../node) `init({ createProject: true })` does — you still need the
CLI installed separately for now.

> **🔒 Security:** `argv` enrichment defaults on and captures the full command line. If your
> app may receive secrets as bare CLI args, call
> `Witslog::init(['enrich' => ['argv' => false]])` — see
> [../CONTRACT.md](../CONTRACT.md#security-note-argv-enrichment-vs-secrets).

## 🧩 Laravel

Auto-discovered via `extra.laravel.providers`. To capture rendered exceptions, in Laravel 11+
`bootstrap/app.php`:

```php
->withExceptions(function (Illuminate\Foundation\Configuration\Exceptions $exceptions) {
    $exceptions->reportable(fn (\Throwable $e) =>
        \Witslog\Witslog::exception(config('app.name', 'laravel'), $e));
})
```

For Laravel 10, call `Witslog::exception(...)` from `App\Exceptions\Handler::report()`.

## 📖 API

| Method | Description |
|--------|--------------|
| `Witslog::init(?array $config)` | Mount the SDK; optionally pass enrich/redact/buffer config. |
| `Witslog::error/warn/info(string $app, string $message, array $opts = [])` | Log at the given severity. `$opts`: `context`, `tags`, `metadata`, `error_code`, `exception`, ... |
| `Witslog::exception(string $app, \Throwable $e, array $opts = [])` | Log a caught exception, capturing `getTraceAsString()`. |
| `Witslog::flush()` / `Witslog::shutdown()` | Drain buffered events before exit. |

## 🌍 Platform support

No bundled native libraries yet — this package has no release CI matrix like
[the Node SDK's](../node) does. Point at a locally built `witslog-ffi` via:

```bash
WITSLOG_LIB=/path/to/witslog_ffi.{dll,so,dylib}
```

or drop the built lib under `_libs/<platform>/` (see [../CONTRACT.md](../CONTRACT.md) for the
platform-dir naming and locator order). Cross-platform prebuilt bundling is tracked for a
future release.

## 🧪 Test

```bash
composer install --ignore-platform-req=ext-ffi
php -d extension=ffi -d ffi.enable=1 vendor/bin/phpunit
```

## 📄 License

Apache License 2.0 — see [../../LICENSE](../../LICENSE).
