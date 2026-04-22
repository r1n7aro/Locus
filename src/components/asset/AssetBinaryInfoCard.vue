<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { AssetBinaryMeta } from "../../types";

const props = defineProps<{
  meta: AssetBinaryMeta;
}>();

function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

const guidShort = computed(() => {
  if (!props.meta.guid) return "—";
  return props.meta.guid.length > 10
    ? `${props.meta.guid.slice(0, 8)}…`
    : props.meta.guid;
});
</script>

<template>
  <div class="abic-root">
    <div class="abic-card">
      <div class="abic-name">{{ meta.name }}</div>
      <div class="abic-row">
        <span class="abic-key">{{ t("asset.preview.binaryInfo.path") }}</span>
        <span class="abic-val">{{ meta.path }}</span>
      </div>
      <div class="abic-row">
        <span class="abic-key">{{ t("asset.preview.binaryInfo.size") }}</span>
        <span class="abic-val">{{ fmtSize(meta.size) }}</span>
      </div>
      <div class="abic-row">
        <span class="abic-key">{{ t("asset.preview.binaryInfo.ext") }}</span>
        <span class="abic-val">{{ meta.ext || "—" }}</span>
      </div>
      <div class="abic-row">
        <span class="abic-key">{{ t("asset.preview.binaryInfo.guid") }}</span>
        <span class="abic-val" :title="meta.guid || ''">{{ guidShort }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.abic-root {
  flex: 1;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 32px;
  overflow: auto;
}
.abic-card {
  min-width: 320px;
  max-width: 560px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  padding: 16px 20px;
  background: var(--panel-bg);
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.abic-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
  margin-bottom: 8px;
  word-break: break-all;
}
.abic-row {
  display: flex;
  align-items: baseline;
  gap: 12px;
  font-size: 12px;
  padding: 4px 0;
}
.abic-key {
  width: 80px;
  flex-shrink: 0;
  color: var(--text-secondary);
}
.abic-val {
  flex: 1;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  word-break: break-all;
}
</style>
