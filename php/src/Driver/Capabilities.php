<?php

declare(strict_types=1);

namespace ImageOxide\Driver;

/**
 * Per-driver capability matrix (PHP-06, AC-PHP-04). Mirrors 003 OPS-02.
 */
final class Capabilities
{
    /**
     * @param array<string, bool> $ops   op name => supported
     * @param array<string, bool> $decode input format => decodable
     * @param array<string, bool> $encode output format => encodable
     * @param array<string, bool> $fits  resize fit modes
     * @param array<string, bool> $positions watermark/resize grid positions
     */
    public function __construct(
        public readonly array $ops,
        public readonly array $decode,
        public readonly array $encode,
        public readonly array $fits = [],
        public readonly array $positions = [],
    ) {
    }
}
