const MAX_U64 = (1n << 64n) - 1n;

export interface NonceSegment {
  start: bigint;
  size: number;
  nonceLength: number;
}

export const nonceLength = (nonce: bigint): number => {
  if (nonce < 0n) {
    throw new Error("nonce must be non-negative");
  }

  if (nonce === 0n) return 1;

  let len = 0;
  let value = nonce;
  while (value > 0n) {
    len += 1;
    value >>= 8n;
  }
  return len;
};

export const cborNonceLength = (nonce: bigint): number => {
  if (nonce < 0n) {
    throw new Error("nonce must be non-negative");
  }

  if (nonce <= 23n) return 1;
  if (nonce <= 0xffn) return 2;
  if (nonce <= 0xffffn) return 3;
  if (nonce <= 0xffff_ffffn) return 5;
  if (nonce <= MAX_U64) return 9;

  throw new Error("nonce range exceeds u64");
};

const maxValueForLength = (len: number): bigint => {
  if (!Number.isInteger(len) || len <= 0 || len > 8) {
    throw new Error("nonceLength must be between 1 and 8");
  }
  return (1n << BigInt(len * 8)) - 1n;
};

const maxValueForCborLength = (len: number): bigint => {
  switch (len) {
    case 1:
      return 23n;
    case 2:
      return 0xffn;
    case 3:
      return 0xffffn;
    case 5:
      return 0xffff_ffffn;
    case 9:
      return MAX_U64;
    default:
      throw new Error("cbor nonceLength must be one of 1, 2, 3, 5, 9");
  }
};

export const splitNonceSegments = (startNonce: bigint, span: number): NonceSegment[] => {
  if (startNonce < 0n) {
    throw new Error("startNonce must be non-negative");
  }
  if (!Number.isInteger(span) || span <= 0) {
    throw new Error("batchSize must be a positive integer");
  }

  const end = startNonce + BigInt(span - 1);
  if (end > MAX_U64) {
    throw new Error("nonce range exceeds u64");
  }

  const segments: NonceSegment[] = [];
  let current = startNonce;

  while (current <= end) {
    const len = nonceLength(current);
    const maxForLen = maxValueForLength(len);
    const segmentEnd = end < maxForLen ? end : maxForLen;
    const segmentSize = segmentEnd - current + 1n;

    if (segmentSize > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new Error("segment size exceeds safe integer range");
    }

    segments.push({
      start: current,
      size: Number(segmentSize),
      nonceLength: len,
    });

    if (segmentEnd === end) {
      break;
    }

    current = segmentEnd + 1n;
  }

  return segments;
};

export const maxNonceForLength = (len: number): bigint => maxValueForLength(len);

export const splitNonceSegmentsCbor = (startNonce: bigint, span: number): NonceSegment[] => {
  if (startNonce < 0n) {
    throw new Error("startNonce must be non-negative");
  }
  if (!Number.isInteger(span) || span <= 0) {
    throw new Error("batchSize must be a positive integer");
  }

  const end = startNonce + BigInt(span - 1);
  if (end > MAX_U64) {
    throw new Error("nonce range exceeds u64");
  }

  const segments: NonceSegment[] = [];
  let current = startNonce;

  while (current <= end) {
    const len = cborNonceLength(current);
    const maxForLen = maxValueForCborLength(len);
    const segmentEnd = end < maxForLen ? end : maxForLen;
    const segmentSize = segmentEnd - current + 1n;

    if (segmentSize > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new Error("segment size exceeds safe integer range");
    }

    segments.push({
      start: current,
      size: Number(segmentSize),
      nonceLength: len,
    });

    if (segmentEnd === end) {
      break;
    }

    current = segmentEnd + 1n;
  }

  return segments;
};


