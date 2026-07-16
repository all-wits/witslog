<?php

declare(strict_types=1);

namespace Witslog\Tests;

use PHPUnit\Framework\TestCase;
use Witslog\Payload;

final class PayloadTest extends TestCase
{
    public function testRequiredFields(): void
    {
        $this->assertSame(
            ['application' => 'app', 'message' => 'msg'],
            Payload::build('app', 'msg')
        );
    }

    public function testPassesContextTagsMetadata(): void
    {
        $p = Payload::build('app', 'msg', [
            'context' => ['request_id' => 'r1'],
            'tags' => ['a', 'b'],
            'metadata' => ['k' => 'v'],
            'severity' => 'warn',
        ]);
        $this->assertSame(['request_id' => 'r1'], $p['context']);
        $this->assertSame(['a', 'b'], $p['tags']);
        $this->assertSame(['k' => 'v'], $p['metadata']);
        $this->assertSame('warn', $p['severity']);
    }

    public function testDropsNullFields(): void
    {
        $p = Payload::build('app', 'msg', ['category' => null, 'tags' => null]);
        $this->assertArrayNotHasKey('category', $p);
        $this->assertArrayNotHasKey('tags', $p);
    }

    public function testRejectsUnknownField(): void
    {
        $this->expectException(\InvalidArgumentException::class);
        Payload::build('app', 'msg', ['bogus' => 1]);
    }

    public function testRejectsNonListTags(): void
    {
        $this->expectException(\InvalidArgumentException::class);
        Payload::build('app', 'msg', ['tags' => ['k' => 'v']]);
    }

    public function testEncodeProducesJson(): void
    {
        $this->assertSame(
            '{"application":"a","message":"m"}',
            Payload::encode(['application' => 'a', 'message' => 'm'])
        );
    }
}
