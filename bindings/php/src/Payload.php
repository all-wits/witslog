<?php

declare(strict_types=1);

namespace Witslog;

/** Pure marshalling — no FFI dependency, unit-testable in isolation. */
final class Payload
{
    public const ALLOWED_FIELDS = [
        'severity',
        'version',
        'environment',
        'category',
        'error_code',
        'exception',
        'stacktrace',
        'correlation_id',
        'parent_event_id',
        'context',
        'tags',
        'metadata',
    ];

    /**
     * Build the witslog_log contract array. Throws on invalid input (FR-P6 error table).
     *
     * @param array<string,mixed> $fields
     * @return array<string,mixed>
     */
    public static function build(string $application, string $message, array $fields = []): array
    {
        $payload = ['application' => $application, 'message' => $message];

        foreach ($fields as $key => $value) {
            if ($value === null) {
                continue;
            }
            if (!in_array($key, self::ALLOWED_FIELDS, true)) {
                throw new \InvalidArgumentException("unknown field: {$key}");
            }
            if ($key === 'tags' && !array_is_list($value)) {
                throw new \InvalidArgumentException('tags must be a list of strings');
            }
            $payload[$key] = $value;
        }

        return $payload;
    }

    /** @param array<string,mixed> $payload */
    public static function encode(array $payload): string
    {
        $json = json_encode($payload, JSON_UNESCAPED_UNICODE);
        if ($json === false) {
            throw new \InvalidArgumentException('payload is not JSON-encodable: ' . json_last_error_msg());
        }
        return $json;
    }
}
