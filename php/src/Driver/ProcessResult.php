<?php

declare(strict_types=1);

namespace ImageOxide\Driver;

/** Result of a driver run: dimensions of the final image and output byte size. */
final class ProcessResult
{
    public function __construct(
        public readonly int $width,
        public readonly int $height,
        public readonly int $bytes,
    ) {
    }
}
