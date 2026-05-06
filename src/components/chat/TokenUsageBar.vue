
<script setup lang="ts">
import { computed } from "vue";
import type { TokenUsage } from "../../types";
import { t } from "../../i18n";

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

const hasPrice = computed(() => props.tokenUsage.pricedRounds > 0);

const contextTokens = computed(() => props.tokenUsage.contextTokens);
const contextLimit = computed(() => props.tokenUsage.contextLimit);
const hasContext = computed(() => contextTokens.value > 0 && contextLimit.value > 0);
const contextPercent = computed(() =>
  contextLimit.value > 0 ? Math.min(100, (contextTokens.value / contextLimit.value) * 100) : 0
);
const contextIndicatorColor = computed(() => {
  const pct = contextPercent.value;
  if (pct >= 80) return "var(--context-danger, #e53e3e)";
  if (pct >= 60) return "var(--context-warning, #d69e2e)";
  return "var(--text-secondary)";
});

const contextTooltip = computed(() => {
  const u = props.tokenUsage;
  const parts = [
    t(
      "chat.tokenUsage.context",
      formatTokens(contextTokens.value),
      formatTokens(contextLimit.value),
      contextPercent.value.toFixed(1),
    ),
  ];
  if (hasPrice.value) {
    parts.push(t("chat.tokenUsage.cost", formatUsd(u.totalCostUsd)));
  }
  return parts.join(" · ");
});

</script>

<template>
  <div
    v-if="hasContext"
    class="token-usage-group"
    role="meter"
    aria-valuemin="0"
    aria-valuemax="100"
    :aria-valuenow="contextPercent.toFixed(1)"
    :aria-label="contextTooltip"
    :aria-valuetext="contextTooltip"
    :style="{ color: contextIndicatorColor }"
    tabindex="0"
  >
    <svg
      class="context-progress-ring"
      viewBox="0 0 16 16"
      aria-hidden="true"
    >
      <circle
        class="context-progress-track"
        cx="8"
        cy="8"
        r="5.2"
        pathLength="100"
      />
      <circle
        class="context-progress-value"
        cx="8"
        cy="8"
        r="5.2"
        pathLength="100"
        :stroke-dasharray="`${contextPercent} 100`"
      />
    </svg>
    <span class="context-usage-label">{{ contextTooltip }}</span>
  </div>
</template>

<style scoped>
.token-usage-group {
  position: relative;
  width: 24px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  align-self: center;
  color: var(--text-secondary);
  cursor: default;
  line-height: 0;
  outline: none;
}

.context-progress-ring {
  width: 15px;
  height: 15px;
  display: block;
  flex-shrink: 0;
  transform: translateY(1px) rotate(-90deg);
}

.context-progress-track,
.context-progress-value {
  fill: none;
  stroke-width: 2;
}

.context-progress-track {
  stroke: color-mix(in srgb, currentColor 28%, transparent);
}

.context-progress-value {
  stroke: currentColor;
  stroke-linecap: round;
  transition: stroke-dasharray 0.2s ease, stroke 0.2s ease;
}

.context-usage-label {
  position: absolute;
  left: 50%;
  bottom: calc(100% + 6px);
  z-index: 35;
  max-width: 240px;
  padding: 4px 7px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: var(--surface-elevated, var(--panel-bg));
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16);
  color: currentColor;
  pointer-events: none;
  overflow: hidden;
  font-size: 11px;
  line-height: 1.3;
  opacity: 0;
  transform: translate(-50%, 3px);
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: opacity 0.1s ease, transform 0.1s ease;
}

.token-usage-group:hover .context-usage-label,
.token-usage-group:focus-visible .context-usage-label {
  opacity: 1;
  transform: translate(-50%, 0);
}

</style>
