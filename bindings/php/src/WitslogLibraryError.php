<?php

declare(strict_types=1);

namespace Witslog;

/** The native witslog library could not be located or loaded. */
class WitslogLibraryError extends WitslogError
{
    /** @var string[] */
    public array $searchedPaths;

    /** @param string[] $searchedPaths */
    public function __construct(array $searchedPaths)
    {
        $this->searchedPaths = $searchedPaths;
        $joined = $searchedPaths ? "\n  " . implode("\n  ", $searchedPaths) : '(none)';
        parent::__construct(
            'could not locate the native witslog library. Set the WITSLOG_LIB '
            . 'environment variable to its path, or bundle it under _libs/<platform>/. '
            . "Searched:{$joined}"
        );
    }
}
