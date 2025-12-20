import { describe, expect, it } from "vitest";
import { nonceLength, splitNonceSegments } from "./nonce";

describe("splitNonceSegments", () => {
  it("splits at the 1->2 byte boundary", () => {
    const segments = splitNonceSegments(0xffn, 2);
    expect(segments).toEqual([
      { start: 0xffn, size: 1, nonceLength: 1 },
      { start: 0x100n, size: 1, nonceLength: 2 },
    ]);
  });

  it("splits at the 2->3 byte boundary", () => {
    const segments = splitNonceSegments(0xffffn, 2);
    expect(segments).toEqual([
      { start: 0xffffn, size: 1, nonceLength: 2 },
      { start: 0x1_0000n, size: 1, nonceLength: 3 },
    ]);
  });

  it("rejects ranges that overflow u64", () => {
    expect(() => splitNonceSegments((1n << 64n) - 1n, 2)).toThrow();
  });
});

describe("nonceLength", () => {
  it("matches minimal byte widths", () => {
    expect(nonceLength(0n)).toBe(1);
    expect(nonceLength(0xffn)).toBe(1);
    expect(nonceLength(0x100n)).toBe(2);
    expect(nonceLength(0xffffn)).toBe(2);
    expect(nonceLength(0x1_0000n)).toBe(3);
  });
});


