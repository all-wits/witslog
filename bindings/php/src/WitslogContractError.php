<?php

declare(strict_types=1);

namespace Witslog;

/** The native library speaks a different ABI/contract version than this SDK. */
class WitslogContractError extends WitslogError
{
    public int $expected;
    public int $actual;

    public function __construct(int $expected, int $actual)
    {
        $this->expected = $expected;
        $this->actual = $actual;
        parent::__construct(
            "witslog contract mismatch: SDK expects ABI version {$expected}, "
            . "native library reports {$actual}. Upgrade the SDK or the native library."
        );
    }
}
