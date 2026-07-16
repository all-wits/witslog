<?php

declare(strict_types=1);

namespace Witslog\Tests;

use PHPUnit\Framework\TestCase;
use Witslog\Ffi;
use Witslog\Witslog;
use Witslog\WitslogContractError;
use Witslog\WitslogLibraryError;
use Witslog\WitslogWriteError;

/** A fake FFI handle: calls dispatch to the closures given per function name. */
final class FakeHandle
{
    /** @param array<string,callable> $fns */
    public function __construct(private array $fns)
    {
    }

    public function __call(string $name, array $args): mixed
    {
        return ($this->fns[$name] ?? fn () => 0)(...$args);
    }
}

final class FfiErrorsTest extends TestCase
{
    protected function tearDown(): void
    {
        // A benign no-op fake, not null: init() in testInitForwardsArgvDisableConfig
        // registers a real PHP shutdown function (register_shutdown_function), which
        // fires after the process exits regardless of test teardown. Leaving a null
        // handle would make that late call fall through to Ffi::load() and fatal on
        // the missing native lib in this test environment.
        Witslog::setHandleForTest(new FakeHandle([]));
    }

    public function testMissingLibraryThrowsLibraryError(): void
    {
        $bogus = DIRECTORY_SEPARATOR . 'no' . DIRECTORY_SEPARATOR . 'such' . DIRECTORY_SEPARATOR . 'witslog_ffi_absent.dll';
        putenv("WITSLOG_LIB={$bogus}");
        try {
            $this->expectException(WitslogLibraryError::class);
            Ffi::load();
        } finally {
            putenv('WITSLOG_LIB');
        }
    }

    public function testCheckAbiThrowsOnMismatch(): void
    {
        $this->expectException(WitslogContractError::class);
        Ffi::checkAbi(999, 1);
    }

    public function testCheckAbiPassesOnMatch(): void
    {
        Ffi::checkAbi(1, 1);
        $this->assertTrue(true);
    }

    public function testWriteErrorOnNegativeReturn(): void
    {
        Witslog::setHandleForTest(new FakeHandle(['witslog_log' => fn ($j) => -1]));
        $this->expectException(WitslogWriteError::class);
        Witslog::log('app', 'boom');
    }

    public function testLogReturnsRowid(): void
    {
        Witslog::setHandleForTest(new FakeHandle(['witslog_log' => fn ($j) => 42]));
        $this->assertSame(42, Witslog::log('app', 'ok', ['context' => ['a' => 1]]));
    }

    public function testInitForwardsArgvDisableConfig(): void
    {
        // The suppression itself is proven natively (witslog-ffi::
        // configure_argv_false_suppresses_argv_capture); this locks that the PHP
        // `init()` surface forwards the config unchanged, so an app that may pass
        // secrets as bare CLI args can fully close that exposure.
        $captured = null;
        Witslog::setHandleForTest(new FakeHandle([
            'witslog_init' => function ($j) use (&$captured) {
                $captured = $j;
                return 0;
            },
        ]));
        Witslog::init(['enrich' => ['argv' => false]]);
        $this->assertStringContainsString('"argv":false', (string) $captured);
    }

    public function testExceptionCapturesTrace(): void
    {
        $captured = null;
        Witslog::setHandleForTest(new FakeHandle([
            'witslog_log' => function ($j) use (&$captured) {
                $captured = $j;
                return 1;
            },
        ]));
        Witslog::exception('app', new \RuntimeException('kaboom'));
        $this->assertStringContainsString('kaboom', (string) $captured);
        $this->assertStringContainsString('stacktrace', (string) $captured);
        $this->assertStringContainsString('RuntimeException', (string) $captured);
    }
}
