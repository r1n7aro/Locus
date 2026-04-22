
<script setup lang="ts">
import { computed } from "vue";
import type { TokenUsage } from "../../types";
import { buildTokenUsageMetrics } from "./tokenUsageDisplay";

const props = defineProps<{
  tokenUsage: TokenUsage;
}>();

function formatTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "k";
  return n.toString();
}

function formatUsd(n: number): string {
  if (n >= 1) return `$${n.toFixed(2)}`;
  if (n >= 0.01) return `$${n.toFixed(4)}`;
  return `$${n.toFixed(6)}`;
}

const tokenMetrics = computed(() => buildTokenUsageMetrics(props.tokenUsage));
const hasPrice = computed(() => props.tokenUsage.pricedRounds > 0);

const contextTokens = computed(() => props.tokenUsage.contextTokens);
const contextLimit = computed(() => props.tokenUsage.contextLimit);
const hasContext = computed(() => contextTokens.value > 0 && contextLimit.value > 0);
const hasVisibleMeta = computed(() => hasContext.value || hasPrice.value);
const contextPercent = computed(() =>
  contextLimit.value > 0 ? Math.min(100, (contextTokens.value / contextLimit.value) * 100) : 0
);
const contextBarColor = computed(() => {
  const pct = contextPercent.value;
  if (pct >= 80) return "var(--context-danger, #e53e3e)";
  if (pct >= 60) return "var(--context-warning, #d69e2e)";
  return "var(--context-normal, #38a169)";
});

const usageTooltip = computed(() => {
  const u = props.tokenUsage;
  let lines: string[] = [];
  if (hasContext.value) {
    lines.push(`上下文: ${formatTokens(contextTokens.value)} / ${formatTokens(contextLimit.value)} (${contextPercent.value.toFixed(1)}%)`);
    lines.push("");
  }
  lines.push("Token 消耗 (含子Agent):");
  for (const metric of tokenMetrics.value) {
    lines.push(`  ${metric.tooltipLabel}: ${metric.value} tokens`);
  }
  if (hasPrice.value) {
    lines.push(`  Cost: ${formatUsd(u.totalCostUsd)}`);
  }
  return lines.join('\n');
});

</script>

<template>
  <div v-if="hasVisibleMeta" class="token-usage-group" :title="usageTooltip">
    <div v-if="hasContext" class="context-usage">
      <span class="context-label">ctx</span>
      <div class="context-bar-track">
        <div
          class="context-bar-fill"
          :style="{ width: contextPercent + '%', background: contextBarColor }"
        />
      </div>
      <span class="context-text">{{ formatTokens(contextTokens) }}<span class="context-sep">/</span>{{ formatTokens(contextLimit) }}</span>
    </div>
    <div v-if="hasPrice" class="token-price">
      <span class="price-label">cost</span>
      <span class="price-total">{{ formatUsd(tokenUsage.totalCostUsd) }}</span>
    </div>
  </div>
</template>
