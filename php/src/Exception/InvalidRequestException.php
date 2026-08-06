<?php

declare(strict_types=1);

namespace ImageOxide\Exception;

/** Malformed frame/JSON or a request sent before ack (IPC-20). */
final class InvalidRequestException extends OxideException
{
}
