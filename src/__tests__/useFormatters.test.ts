import { describe, it, expect } from "vitest";
import { formatTokens, formatUsd } from "../composables/useFormatters";

describe("formatTokens", () => {
  it("formats millions", () => {
    expect(formatTokens(1_500_000)).toBe("1.5M");
    expect(formatTokens(1_000_000)).toBe("1.0M");
    expect(formatTokens(10_000_000)).toBe("10.0M");
  });

  it("formats thousands", () => {
    expect(formatTokens(1_500)).toBe("1.5k");
    expect(formatTokens(1_000)).toBe("1.0k");
    expect(formatTokens(999_999)).toBe("1000.0k");
  });

  it("formats small numbers as-is", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(1)).toBe("1");
    expect(formatTokens(999)).toBe("999");
  });
});

describe("formatUsd", () => {
  it("formats >= $1 with 2 decimals", () => {
    expect(formatUsd(1)).toBe("$1.00");
    expect(formatUsd(10.5)).toBe("$10.50");
    expect(formatUsd(123.456)).toBe("$123.46");
  });

  it("formats >= $0.01 with 4 decimals", () => {
    expect(formatUsd(0.01)).toBe("$0.0100");
    expect(formatUsd(0.1234)).toBe("$0.1234");
    expect(formatUsd(0.99)).toBe("$0.9900");
  });

  it("formats tiny amounts with 6 decimals", () => {
    expect(formatUsd(0)).toBe("$0.000000");
    expect(formatUsd(0.001)).toBe("$0.001000");
    expect(formatUsd(0.009999)).toBe("$0.009999");
  });
});
