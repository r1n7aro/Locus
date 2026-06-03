<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { AssetSearchResult } from "../../types";

const props = defineProps<{
  results: AssetSearchResult[];
  query: string;
  searching: boolean;
  hasFallback: boolean;
  truncated: boolean;
  selectedPath?: string | null;
  selectedKey?: string | null;
}>();

const emit = defineEmits<{
  (e: "select", result: AssetSearchResult): void;
}>();

// Parse query into:
//  - text terms: bare substrings or n=/n^/n$/n: values, used to highlight
//    file name & path
//  - kind terms: t:xxx values, used to highlight the kind tag
// Other predicates (under:, guid:) are ignored for highlighting.
interface ParsedQuery {
  textTerms: string[];
  kindTerms: string[];
}

const parsedQuery = computed<ParsedQuery>(() => {
  const raw = (props.query || "").trim();
  if (!raw) return { textTerms: [], kindTerms: [] };
  const text: string[] = [];
  const kinds: string[] = [];
  for (const tok of raw.split(/\s+/)) {
    if (!tok) continue;
    const m = /^([a-z]+)([:=^$])(.*)$/i.exec(tok);
    if (m) {
      const [, key, , val] = m;
      const k = key.toLowerCase();
      if (k === "t") {
        if (val) kinds.push(val.toLowerCase());
        continue;
      }
      if (k === "under" || k === "guid") continue;
      if (k === "n") {
        if (val) text.push(val.toLowerCase());
        continue;
      }
      continue;
    }
    text.push(tok.toLowerCase());
  }
  return {
    textTerms: [...new Set(text)].sort((a, b) => b.length - a.length),
    kindTerms: [...new Set(kinds)],
  };
});

function kindMatches(kind: string): boolean {
  const terms = parsedQuery.value.kindTerms;
  if (terms.length === 0) return false;
  const k = (kind || "").toLowerCase();
  return terms.some((t) => k.includes(t));
}

function escapeRe(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const highlightRe = computed<RegExp | null>(() => {
  const terms = parsedQuery.value.textTerms;
  if (terms.length === 0) return null;
  return new RegExp(`(${terms.map(escapeRe).join("|")})`, "gi");
});

interface Segment {
  text: string;
  hit: boolean;
}

function segments(text: string): Segment[] {
  const re = highlightRe.value;
  if (!re || !text) return [{ text, hit: false }];
  const result: Segment[] = [];
  let last = 0;
  re.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) result.push({ text: text.slice(last, m.index), hit: false });
    result.push({ text: m[0], hit: true });
    last = m.index + m[0].length;
    if (m[0].length === 0) re.lastIndex++;
  }
  if (last < text.length) result.push({ text: text.slice(last), hit: false });
  return result;
}

function resultKey(result: AssetSearchResult): string {
  if (result.objectKey) return result.objectKey;
  if (result.isSubAsset && result.name.trim()) return `${result.path}/${result.name.trim()}`;
  return result.path;
}

function resultDisplayPath(result: AssetSearchResult): string {
  if (result.isSubAsset && result.name.trim()) return `${result.path}/${result.name.trim()}`;
  return result.path;
}
</script>

<template>
  <div class="asr-list">
    <div v-if="hasFallback" class="asr-hint asr-hint-warn">
      {{ t("asset.search.indexNotReady") }}
    </div>
    <div v-if="truncated" class="asr-hint">
      {{ t("asset.search.truncated") }}
    </div>
    <div v-if="searching && results.length === 0" class="asr-empty">…</div>
    <div v-else-if="!searching && results.length === 0" class="asr-empty">
      {{ t("asset.search.empty") }}
    </div>
    <button
      v-for="(r, i) in results"
      :key="resultKey(r)"
      type="button"
      class="asr-row"
      :class="{ selected: selectedKey ? selectedKey === resultKey(r) : selectedPath === r.path }"
      @click="emit('select', r)"
      :title="resultDisplayPath(r)"
    >
      <span class="asr-index">{{ i + 1 }}</span>
      <span class="asr-name">
        <template v-for="(seg, si) in segments(r.name)" :key="si"
          ><mark v-if="seg.hit" class="asr-hit">{{ seg.text }}</mark
          ><template v-else>{{ seg.text }}</template></template>
      </span>
      <span
        class="asr-kind"
        :class="{ 'asr-kind-hit': kindMatches(r.kind) }"
        :title="r.typeLabel ? `${r.typeLabel} (${r.kind})` : r.kind"
      >
        <template v-if="r.typeLabel"
          ><span v-if="!r.isSubAsset" class="asr-kind-prefix">SO:</span
          ><template v-for="(seg, si) in segments(r.typeLabel)" :key="si"
            ><mark v-if="seg.hit" class="asr-hit">{{ seg.text }}</mark
            ><template v-else>{{ seg.text }}</template></template
          ></template
        >
        <template v-else
          ><template v-for="(seg, si) in segments(r.kind)" :key="si"
            ><mark v-if="seg.hit" class="asr-hit">{{ seg.text }}</mark
            ><template v-else>{{ seg.text }}</template></template
          ></template
        >
      </span>
      <span class="asr-path">
        <template v-for="(seg, si) in segments(resultDisplayPath(r))" :key="si"
          ><mark v-if="seg.hit" class="asr-hit">{{ seg.text }}</mark
          ><template v-else>{{ seg.text }}</template></template>
      </span>
    </button>
  </div>
</template>

<style scoped>
.asr-list {
  flex: 1;
  overflow-y: auto;
  padding: 0;
}
.asr-hint {
  padding: 6px 12px;
  font-size: 11px;
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--hover-bg) 22%);
  border-bottom: 1px solid var(--border-color);
}
.asr-hint-warn {
  color: var(--status-warn-fg);
}
.asr-empty {
  padding: 16px 12px;
  text-align: center;
  font-size: 12px;
  color: var(--text-secondary);
}
.asr-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 6px 12px 6px 6px;
  border: none;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  text-align: left;
  cursor: pointer;
  border-bottom: 1px solid var(--border-color);
  overflow: hidden;
}
.asr-row:hover { background: var(--hover-bg); }
.asr-row.selected,
.asr-row.selected:hover {
  background: var(--active-bg);
}
.asr-index {
  flex-shrink: 0;
  min-width: 22px;
  text-align: right;
  font-size: 10px;
  font-variant-numeric: tabular-nums;
  color: var(--text-secondary);
  opacity: 0.55;
  font-family: var(--font-mono-identifier);
}
.asr-name {
  font-weight: 500;
  flex-shrink: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 240px;
}
.asr-kind {
  flex-shrink: 0;
  max-width: 240px;
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 3px;
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--hover-bg) 26%);
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.asr-kind-prefix {
  opacity: 0.6;
  margin-right: 2px;
}
.asr-hit {
  background: rgba(255, 196, 0, 0.28);
  color: inherit;
  border-radius: 2px;
  padding: 0 1px;
}
.asr-kind-hit {
  background: color-mix(in srgb, var(--status-warn-bg) 72%, var(--panel-bg) 28%);
  color: var(--text-color);
}
.asr-path {
  flex: 1;
  min-width: 0;
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--font-mono-identifier);
}
</style>
