import { ZeldMinerErrorCode, type ZeldMinerErrorDetails } from "./types";

const formatUnknownError = (err: unknown): string =>
  err instanceof Error ? err.message : String(err);

export class ZeldMinerError extends Error {
  readonly code: ZeldMinerErrorCode;
  readonly details?: ZeldMinerErrorDetails;

  constructor(
    code: ZeldMinerErrorCode,
    message: string,
    details?: ZeldMinerErrorDetails
  ) {
    super(message);
    this.name = "ZeldMinerError";
    this.code = code;
    this.details = details;
  }
}

export const toZeldMinerError = (
  err: unknown,
  fallbackCode: ZeldMinerErrorCode = ZeldMinerErrorCode.WORKER_ERROR,
  details?: ZeldMinerErrorDetails
): ZeldMinerError => {
  if (err instanceof ZeldMinerError) {
    return err;
  }

  const message = formatUnknownError(err);
  return new ZeldMinerError(fallbackCode, message, details);
};

export const createMinerError = (
  code: ZeldMinerErrorCode,
  message: string,
  details?: ZeldMinerErrorDetails
): ZeldMinerError => new ZeldMinerError(code, message, details);

