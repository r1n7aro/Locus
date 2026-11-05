<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, shallowRef, watch } from "vue";
import { documentSearchRanges } from "../../composables/documentSearch";
import { isSearchScopeVisible, useSearchScope } from "../../composables/useSearchScope";
import FindBar from "./FloatingFindBar.vue";

const props = defineProps<{ textSelector?: string }>();
const root = ref<HTMLElement | null>(null);
const content = ref<HTMLElement | null>(null);
const bar = ref<InstanceType<typeof FindBar> | null>(null);
const visible = ref(false);
const query = ref("");
const matches = shallowRef<Range[]>([]);
const position = ref(0);
const current = computed(() => matches.value.length ? position.value + 1 : 0);
let previousFocus: HTMLElement | null = null;
let observer: MutationObserver | null = null;
let releaseHighlights: (() => void) | undefined;
let searchEpoch = 0;
// Names are unique across Vue app roots sharing one window.
const highlightId = `locus-find-${crypto.randomUUID()}`;

function highlight() {
  releaseHighlights?.();
  releaseHighlights = undefined;
  const owner = root.value?.ownerDocument.defaultView;
  if (!owner || !visible.value) return;
  const api = owner as unknown as {
    CSS?: { highlights?: Map<string, unknown> };
    Highlight?: new (...ranges: Range[]) => { add(range: Range): unknown };
  };
  if (!api.CSS?.highlights || !api.Highlight) return;
  const registry = api.CSS.highlights;
  const currentName = `${highlightId}-current`;
  const all = new api.Highlight();
  for (const range of matches.value) all.add(range);
  registry.set(highlightId, all);
  const match = matches.value[position.value];
  if (match) registry.set(currentName, new api.Highlight(match));
  const style = owner.document.createElement("style");
  style.textContent = `::highlight(${highlightId}) { background-color: var(--find-match-bg); color: var(--text-color); } ::highlight(${currentName}) { background-color: var(--find-match-current-bg); color: var(--text-color); }`;
  owner.document.head.append(style);
  releaseHighlights = () => { registry.delete(highlightId); registry.delete(currentName); style.remove(); };
}
function reveal() {
  highlight();
  const range = matches.value[position.value];
  if (!range || !root.value || !isSearchScopeVisible(root.value)) return;
  range.startContainer.parentElement?.scrollIntoView?.({ block: "center", inline: "nearest" });
}
function refresh() {
  if (!visible.value || !content.value) return;
  matches.value = documentSearchRanges(content.value, query.value, props.textSelector);
  position.value = Math.min(position.value, Math.max(0, matches.value.length - 1));
  reveal();
}
async function open() {
  if (!visible.value) {
    const active = root.value?.ownerDocument.activeElement as HTMLElement | null;
    previousFocus = root.value?.contains(active) ? active : root.value;
  }
  visible.value = true;
  await nextTick();
  bar.value?.focus();
  refresh();
}
function close() {
  visible.value = false;
  releaseHighlights?.();
  releaseHighlights = undefined;
  const target = previousFocus && isSearchScopeVisible(previousFocus) ? previousFocus : root.value;
  target?.focus({ preventScroll: true });
}
function move(delta: number) {
  if (!matches.value.length) return;
  position.value = (position.value + delta + matches.value.length) % matches.value.length;
  reveal();
}
useSearchScope(root, () => { void open(); });
watch(query, () => { position.value = 0; refresh(); });
watch([content, visible], ([element, open]) => {
  observer?.disconnect();
  if (!element || !open) return;
  observer = new MutationObserver(() => {
    const epoch = ++searchEpoch;
    void nextTick(() => { if (epoch === searchEpoch) refresh(); });
  });
  observer.observe(element, { subtree: true, childList: true, characterData: true });
});
onBeforeUnmount(() => { searchEpoch += 1; observer?.disconnect(); releaseHighlights?.(); });
</script>

<template>
  <div ref="root" class="searchable-content" tabindex="-1">
    <FindBar v-if="visible" ref="bar" :anchor="root" v-model:query="query" :count="matches.length" :current="current"
      @next="move(1)" @previous="move(-1)" @close="close" />
    <div ref="content" class="searchable-content-body"><slot /></div>
  </div>
</template>

<style scoped>
.searchable-content { display: flex; flex: 1; flex-direction: column; min-width: 0; min-height: 0; overflow: hidden; outline: none; }
.searchable-content-body { display: flex; flex: 1; flex-direction: column; min-height: 0; overflow: auto; }
</style>
