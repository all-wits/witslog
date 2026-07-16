<?php

declare(strict_types=1);

namespace Witslog\Laravel;

use Witslog\Witslog;

/**
 * Laravel service provider. Auto-discovered via composer `extra.laravel.providers`.
 *
 * Config (config/witslog.php or env):
 *   WITSLOG_APPLICATION  — app name (default: the Laravel app name, else 'laravel')
 *
 * To capture rendered exceptions in Laravel 11+, add to bootstrap/app.php:
 *
 *   ->withExceptions(function (Illuminate\Foundation\Configuration\Exceptions $exceptions) {
 *       $exceptions->reportable(fn (\Throwable $e) => \Witslog\Witslog::exception(
 *           config('app.name', 'laravel'), $e
 *       ));
 *   })
 *
 * For Laravel 10, call Witslog::exception(...) from App\Exceptions\Handler::report().
 */
class WitslogServiceProvider extends \Illuminate\Support\ServiceProvider
{
    public function boot(): void
    {
        $application = env('WITSLOG_APPLICATION', config('app.name', 'laravel'));

        // Mount once. A native/lib failure must never take down the host app.
        try {
            Witslog::init();
        } catch (\Throwable) {
            return;
        }

        // Flush buffered events when the framework terminates a request/command.
        if (method_exists($this->app, 'terminating')) {
            $this->app->terminating(static function (): void {
                try {
                    Witslog::flush();
                } catch (\Throwable) {
                    // best-effort flush
                }
            });
        }

        // Expose the app name so callers can default to it.
        $this->app->instance('witslog.application', $application);
    }
}
