import type { TokenUsage } from "../../types";

export interface TokenUsageMetric {
  key: "input" | "uncached-input" | "cached-input-write" | "cached-input-read" | "output";
  shortLabel: string;
  tooltipLabel: string;
  value: number;
}

export function buildTokenUsageMetrics(usage: TokenUsage): TokenUsageMetric[] {
  const hasCache = usage.totalCacheReadTokens > 0 || usage.totalCacheWriteTokens > 0;

  if (!hasCache) {
    return [
      {
        key: "input",
        shortLabel: "input",
        tooltipLabel: "input",
        value: usage.totalInputTokens,
      },
      {
        key: "output",
        shortLabel: "output",
        tooltipLabel: "output",
        value: usage.totalOutputTokens,
      },
    ];
  }

  return [
    {
      key: "uncached-input",
      shortLabel: "uncached",
      tooltipLabel: "uncached input",
      value: usage.totalInputTokens,
    },
    {
      key: "cached-input-write",
      shortLabel: "cache write",
      tooltipLabel: "cached input write",
      value: usage.totalCacheWriteTokens,
    },
    {
      key: "cached-input-read",
      shortLabel: "cache read",
      tooltipLabel: "cached input read",
      value: usage.totalCacheReadTokens,
    },
    {
      key: "output",
      shortLabel: "output",
      tooltipLabel: "output",
      value: usage.totalOutputTokens,
    },
  ];
}
