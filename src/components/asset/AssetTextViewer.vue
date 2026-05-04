<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import hljs from "../../hljs";

const props = defineProps<{
  snippet: string;
  truncated: boolean;
  totalLines: number;
  language?: string;
}>();

const lines = computed(() => props.snippet.split("\n"));
const shownLines = computed(() => lines.value.length);
const languageClass = computed(() => (props.language ? `language-${props.language}` : null));
const highlightedLines = computed(() => {
  let highlighted: string;
  const language = props.language;
  if (language && hljs.getLanguage(language)) {
    try {
      highlighted = hljs.highlight(props.snippet, { language }).value;
    } catch {
      highlighted = escapeHtml(props.snippet);
    }
  } else {
    highlighted = escapeHtml(props.snippet);
  }
  return highlighted.split("\n");
});

function escapeHtml(source: string): string {
  return source.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
</script>

<template>
  <div class="atv-root">
    <div class="atv-body">
      <pre class="atv-pre hljs" :class="languageClass"><code><span
        v-for="(line, i) in highlightedLines"
        :key="i"
        class="atv-line"
      ><span class="atv-ln">{{ i + 1 }}</span><span class="atv-text" v-html="line || ' '"></span>
</span></code></pre>
    </div>
    <div v-if="truncated" class="atv-footer">
      {{ t("asset.preview.truncated", shownLines) }}
    </div>
  </div>
</template>

<style scoped>
.atv-root {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}
.atv-body {
  flex: 1;
  overflow: auto;
  background: var(--panel-bg);
}
.atv-pre.hljs {
  margin: 0;
  padding: 8px 0;
  font-family: var(--font-mono-editor);
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-color);
  background: var(--panel-bg);
  white-space: pre;
}
.atv-line {
  display: flex;
}
.atv-ln {
  flex-shrink: 0;
  width: 48px;
  padding-right: 12px;
  text-align: right;
  color: var(--text-secondary);
  user-select: none;
  opacity: 0.6;
}
.atv-text {
  flex: 1;
  white-space: pre;
}
.atv-footer {
  padding: 6px 12px;
  font-size: 11px;
  color: var(--text-secondary);
  border-top: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--hover-bg) 22%);
  flex-shrink: 0;
}
</style>
