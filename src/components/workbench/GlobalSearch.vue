<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, useId, watch } from "vue";
import { Search, Settings2, X } from "lucide";
import { locale, t } from "../../i18n";
import { formatRelativeDate } from "../../composables/useFormatters";
import { useGlobalSearch } from "../../composables/useGlobalSearch";
import { useGlobalSearchSettings } from "../../composables/useGlobalSearchSettings";
import { globalSearchHitKey, searchHighlightParts, type GlobalSearchResult, type GlobalSearchTarget } from "../../services/globalSearch";
import { normalizeAppError } from "../../services/errors";
import BaseButton from "../ui/BaseButton.vue";
import LucideIcon from "../icons/LucideIcon.vue";

const props = defineProps<{
  active: boolean;
  targets: GlobalSearchTarget[];
  ownerWindow?: Window;
  openResult: (result: GlobalSearchResult) => Promise<void>;
}>();
const emit = defineEmits<{ settings: [] }>();
const owner = props.ownerWindow ?? window;
const { state: settings } = useGlobalSearchSettings();
const { results, searching, hasMore, errors, search, cancel, loadMore, empty } = useGlobalSearch();
const visible = ref(false);
const query = ref("");
const composing = ref(false);
const opening = ref(false);
const openError = ref("");
const input = ref<HTMLInputElement | null>(null);
const panel = ref<HTMLElement | null>(null);
const scroll = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const selectedKey = ref("");
const timeNow = ref(Date.now());
let timeRefreshTimer: number | undefined;
const absoluteTimeFormatter = computed(() => new Intl.DateTimeFormat(locale.value === "zh" ? "zh-CN" : "en-US", {
  year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit",
}));
const listId = useId();
let previousFocus: HTMLElement | null = null;
const keyOf = (hit: GlobalSearchResult) => hit.target.projectId + globalSearchHitKey(hit);
const selectedIndex = computed(() => Math.max(0, results.value.findIndex((hit) => keyOf(hit) === selectedKey.value)));
const selected = computed(() => results.value[selectedIndex.value]);
const hasScope = computed(() => settings.knowledgeTitle || settings.knowledgeContent || settings.sessionTitle || settings.sessionContent);
const showProjectName = computed(() => new Set(props.targets.map((target) => target.projectId)).size > 1);
const MAX_RESULTS_HEIGHT = 432;
const RESULT_HEIGHT = 54;
const LOAD_MORE_THRESHOLD = 96;
const resultLayout = computed(() => ({
  rows: results.value.map((hit, index) => ({ hit, index, top: index * RESULT_HEIGHT, height: RESULT_HEIGHT })),
  totalHeight: results.value.length * RESULT_HEIGHT,
}));
function rowAtOffset(rows: typeof resultLayout.value.rows, offset: number): number {
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    const row = rows[middle]!;
    if (row.top + row.height <= offset) low = middle + 1;
    else high = middle;
  }
  return Math.min(low, Math.max(0, rows.length - 1));
}
const visibleRows = computed(() => {
  const rows = resultLayout.value.rows;
  const start = Math.max(0, rowAtOffset(rows, scrollTop.value) - 2);
  const end = rowAtOffset(rows, scrollTop.value + MAX_RESULTS_HEIGHT) + 3;
  return rows.slice(start, end);
});
const rowsHeight = computed(() => Math.min(MAX_RESULTS_HEIGHT, resultLayout.value.totalHeight));
const activeDescendant = computed(() => visibleRows.value.some((row) => row.index === selectedIndex.value)
  ? `${listId}-${selectedIndex.value}` : undefined);
function categoryLabel(hit: GlobalSearchResult): string {
  if (hit.kind === "session") return t("globalSearch.session");
  return hit.docType ? t(`knowledge.type.${hit.docType}`) : t("globalSearch.knowledge");
}
const resultParts = computed(() => visibleRows.value.map((row) => {
  const date = new Date(row.hit.modifiedAt);
  const validDate = row.hit.modifiedAt > 0 && Number.isFinite(date.getTime());
  const relativeTime = validDate
    ? (timeNow.value - row.hit.modifiedAt < 60_000 ? t("time.justNow") : formatRelativeDate(row.hit.modifiedAt / 1000))
    : "—";
  const timeTitle = validDate ? t("globalSearch.modifiedAt", absoluteTimeFormatter.value.format(date)) : "";
  return {
    ...row,
    category: row.hit.kind === "knowledge" && row.hit.docType === "reference" ? "REF" : categoryLabel(row.hit),
    relativeTime,
    timeTitle,
    dateTime: validDate ? date.toISOString() : undefined,
    titleParts: searchHighlightParts(row.hit.title, query.value, 12),
    snippetParts: row.hit.excerpt.trim()
      ? searchHighlightParts(row.hit.excerpt, query.value, 24)
      : [{ text: t("globalSearch.noPreview"), match: false }],
    details: [row.hit.target.projectName, categoryLabel(row.hit), row.hit.path, row.hit.archived ? t("globalSearch.archived") : "", timeTitle].filter(Boolean).join(" · "),
  };
}));
const targetKey = computed(() => JSON.stringify(props.targets));

function maybeLoadMore() {
  const element = scroll.value;
  if (!visible.value || !element || element.clientHeight <= 0 || !hasMore.value || searching.value) return;
  if (resultLayout.value.totalHeight - element.scrollTop - element.clientHeight <= LOAD_MORE_THRESHOLD) loadMore();
}
function onScroll() {
  scrollTop.value = scroll.value?.scrollTop ?? 0;
  maybeLoadMore();
}
watch([searching, hasMore, () => resultLayout.value.totalHeight], () => {
  void nextTick(maybeLoadMore);
}, { flush: "post" });

function runSearch() {
  openError.value = "";
  selectedKey.value = "";
  scrollTop.value = 0;
  if (scroll.value) scroll.value.scrollTop = 0;
  if (visible.value && !composing.value) search(query.value, props.targets, { ...settings });
  else cancel();
}
watch([query, targetKey, () => ({ ...settings }), composing], runSearch, { flush: "sync" });
watch(() => props.active && settings.enabled, (active) => { if (!active) close(false); });
watch(visible, (open) => {
  owner.clearInterval(timeRefreshTimer);
  if (open) {
    timeNow.value = Date.now();
    timeRefreshTimer = owner.setInterval(() => { timeNow.value = Date.now(); }, 60_000);
  }
});

async function show() {
  if (!visible.value) {
    previousFocus = owner.document.activeElement as HTMLElement | null;
    visible.value = true;
    runSearch();
  }
  await nextTick();
  input.value?.focus();
  input.value?.select();
}
function close(restoreFocus = true) {
  if (!visible.value) return;
  visible.value = false;
  cancel();
  if (restoreFocus && previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
}
function openSettings() { close(false); emit("settings"); }
async function select(hit = selected.value) {
  if (!hit || opening.value || composing.value) return;
  opening.value = true;
  openError.value = "";
  try { await props.openResult(hit); close(false); }
  catch (error) { openError.value = normalizeAppError(error).message; }
  finally { opening.value = false; }
}
function selectIndex(index: number) {
  const hit = results.value[index];
  if (!hit) return;
  selectedKey.value = keyOf(hit);
  if (!scroll.value) return;
  const { top, height } = resultLayout.value.rows[index]!;
  if (top < scroll.value.scrollTop) scroll.value.scrollTop = top;
  else if (top + height > scroll.value.scrollTop + scroll.value.clientHeight) {
    scroll.value.scrollTop = top + height - scroll.value.clientHeight;
  }
  onScroll();
}
function move(delta: number) {
  if (!results.value.length) return;
  selectIndex((selectedIndex.value + delta + results.value.length) % results.value.length);
}
function movePage(direction: number) {
  const rows = resultLayout.value.rows;
  const row = rows[selectedIndex.value];
  if (!row) return;
  const pageHeight = scroll.value?.clientHeight || MAX_RESULTS_HEIGHT;
  selectIndex(rowAtOffset(rows, Math.max(0, row.top + direction * pageHeight)));
}
function onKeydown(event: KeyboardEvent) {
  if (!props.active || !settings.enabled || event.isComposing || event.keyCode === 229) return;
  if ((event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "f") {
    event.preventDefault(); event.stopImmediatePropagation(); void show(); return;
  }
  if (!visible.value) return;
  if (event.key === "Escape") {
    event.preventDefault(); event.stopImmediatePropagation(); close(); return;
  }
  if (event.target !== input.value) return;
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault(); event.stopImmediatePropagation(); move(event.key === "ArrowDown" ? 1 : -1);
  } else if (event.key === "PageDown" || event.key === "PageUp") {
    event.preventDefault(); event.stopImmediatePropagation(); movePage(event.key === "PageDown" ? 1 : -1);
  } else if ((event.ctrlKey || event.metaKey) && (event.key === "Home" || event.key === "End")) {
    event.preventDefault(); event.stopImmediatePropagation(); selectIndex(event.key === "Home" ? 0 : results.value.length - 1);
  } else if (event.key === "Enter") {
    event.preventDefault(); event.stopImmediatePropagation(); void select();
  }
}
function onOutside(event: Event) {
  if (visible.value && !event.composedPath().includes(panel.value!)) close(false);
}
onMounted(() => {
  owner.addEventListener("keydown", onKeydown, true);
  owner.addEventListener("pointerdown", onOutside, true);
  owner.addEventListener("focusin", onOutside);
});
onUnmounted(() => {
  cancel();
  owner.clearInterval(timeRefreshTimer);
  owner.removeEventListener("keydown", onKeydown, true);
  owner.removeEventListener("pointerdown", onOutside, true);
  owner.removeEventListener("focusin", onOutside);
});
</script>

<template>
  <Teleport :to="owner.document.body">
    <section v-if="visible" ref="panel" class="global-search" role="dialog" :aria-label="t('settings.tab.globalSearch')">
      <div class="search-input-row">
        <LucideIcon :icon="Search" :size="18" />
        <input ref="input" v-model="query" class="search-input" type="text" maxlength="200" autocomplete="off" spellcheck="false"
          role="combobox" aria-autocomplete="list" :aria-expanded="!!results.length && !!query.trim()" :aria-controls="results.length ? listId : undefined"
          :aria-activedescendant="activeDescendant"
          :aria-label="t('settings.tab.globalSearch')" :placeholder="t('globalSearch.placeholder')"
          @compositionstart="composing = true" @compositionend="composing = false" />
        <BaseButton class="search-icon-button" :aria-label="t('globalSearch.settings')" :title="t('globalSearch.settings')" @click="openSettings">
          <LucideIcon :icon="Settings2" :size="16" />
        </BaseButton>
        <BaseButton class="search-icon-button" :aria-label="t('globalSearch.close')" :title="t('globalSearch.close')" @click="close()">
          <LucideIcon :icon="X" :size="18" />
        </BaseButton>
      </div>
      <div v-if="query.trim() || !targets.length || !hasScope" class="search-results">
        <div v-if="!targets.length" class="search-state">{{ t('globalSearch.noWorkspace') }}</div>
        <div v-else-if="!hasScope" class="search-state">{{ t('globalSearch.noScope') }}</div>
        <template v-else>
          <div v-if="results.length" ref="scroll" class="search-scroll" :style="{ height: `${rowsHeight}px` }" @scroll="onScroll">
            <div :id="listId" role="listbox" :aria-label="t('globalSearch.results')" :aria-busy="searching" class="search-list" :style="{ height: `${resultLayout.totalHeight}px` }">
              <button v-for="row in resultParts" :id="`${listId}-${row.index}`" :key="keyOf(row.hit)" class="search-result" role="option" type="button" tabindex="-1"
                :title="`${row.hit.title}\n${row.details}`" :aria-label="`${row.hit.title} · ${row.details}`" :aria-describedby="`${listId}-${row.index}-snippet`"
                :aria-selected="row.index === selectedIndex" :disabled="opening" :style="{ transform: `translateY(${row.top}px)`, height: `${row.height}px` }"
                @mousemove="selectedKey = keyOf(row.hit)" @mousedown.left.prevent @click="select(row.hit)">
                <span class="search-result-category">{{ row.category }}</span>
                <span class="search-result-text">
                  <span class="search-result-heading">
                    <span class="search-result-title"><template v-for="(part, index) in row.titleParts" :key="index"><mark v-if="part.match">{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></span>
                    <span class="search-result-trailing">
                      <span v-if="showProjectName" class="search-result-meta search-result-project">{{ row.hit.target.projectName }}</span>
                      <span v-if="row.hit.archived" class="search-result-meta search-result-archived">{{ t('globalSearch.archived') }}</span>
                      <time class="search-result-time" :datetime="row.dateTime" :title="row.timeTitle">{{ row.relativeTime }}</time>
                    </span>
                  </span>
                  <span :id="`${listId}-${row.index}-snippet`" class="search-result-snippet"><template v-for="(part, index) in row.snippetParts" :key="index"><mark v-if="part.match">{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></span>
                </span>
              </button>
            </div>
          </div>
          <div v-if="empty" class="search-state" role="status">{{ t('globalSearch.noResults') }}</div>
          <div v-if="searching" class="search-status" role="status">{{ t('globalSearch.searching') }}</div>
          <div v-if="errors.length || openError" class="search-error" role="alert">{{ openError || errors.join('\n') }}</div>
        </template>
      </div>
    </section>
  </Teleport>
</template>

<style scoped>
.global-search { position: fixed; z-index: 1800; top: 48px; left: 50%; transform: translateX(-50%); width: min(760px, calc(100vw - 32px)); max-height: calc(100vh - 72px); overflow: hidden; display: flex; flex-direction: column; border: 1px solid var(--border-strong); border-radius: 8px; background: var(--panel-bg); box-shadow: 0 8px 28px color-mix(in srgb, var(--text-color) 12%, transparent); color: var(--text-color); }
.search-input-row { display: flex; align-items: center; gap: 10px; flex: none; min-height: 48px; padding: 0 10px 0 14px; color: var(--text-secondary); }
.search-input { flex: 1; width: 0; border: none; outline: none; background: transparent; color: var(--text-color); font: inherit; font-size: 14px; }
.search-input::placeholder { color: var(--text-secondary); }
.search-icon-button { width: 28px; height: 28px; padding: 4px; border-color: transparent; }
.search-icon-button:focus-visible { outline: 1px solid var(--accent-color); }
.search-results { display: flex; flex-direction: column; min-height: 0; overflow: hidden; border-top: 1px solid var(--border-color); }
.search-scroll { overflow-x: hidden; overflow-y: auto; min-height: 0; flex: 0 1 auto; overscroll-behavior: contain; overflow-anchor: none; }
.search-list { position: relative; }
.search-result { position: absolute; top: 0; left: 0; display: grid; grid-template-columns: 72px minmax(0, 1fr); gap: 14px; align-items: center; box-sizing: border-box; width: 100%; padding: 9px 14px; border: none; background: transparent; color: var(--text-color); font-family: var(--font-ui); text-align: left; cursor: pointer; }
.search-result:hover { background: var(--hover-bg); }
.search-result[aria-selected="true"] { background: var(--active-bg); }
.search-result:focus-visible { outline: 1px solid var(--accent-color); outline-offset: -2px; }
.search-result:disabled { cursor: progress; }
.search-result-category { box-sizing: border-box; min-width: 0; padding: 0 5px; border: 1px solid var(--border-strong); border-radius: 4px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; font-size: 11px; font-weight: 600; line-height: 18px; text-align: center; text-transform: uppercase; letter-spacing: 0.02em; color: var(--text-secondary); }
.search-result-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.search-result-heading { display: flex; align-items: center; gap: 16px; min-width: 0; }
.search-result-title, .search-result-snippet, .search-result-meta, .search-result-time { display: block; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; line-height: 18px; }
.search-result-title { flex: 1; min-width: 0; font-size: 13px; font-weight: 500; }
.search-result-snippet { font-size: 12px; line-height: 16px; color: var(--text-secondary); }
.search-result-trailing { display: flex; align-items: center; justify-content: flex-end; gap: 12px; flex: 0 1 auto; min-width: 0; max-width: 38%; }
.search-result-meta { flex: 0 1 auto; min-width: 0; font-size: 11px; color: var(--text-secondary); text-align: right; }
.search-result-archived { flex: none; }
.search-result-time { flex: 0 0 6em; width: 6em; font-size: 11px; font-variant-numeric: tabular-nums; color: var(--text-secondary); text-align: right; }
mark { background: transparent; color: var(--accent-color); font-weight: 600; }
.search-state { padding: 20px 14px; color: var(--text-secondary); font-size: 13px; }
.search-status, .search-error { flex: none; padding: 8px 14px; font-size: 12px; }
.search-status { color: var(--text-secondary); }
.search-error { color: var(--status-danger-fg); max-height: 80px; overflow: auto; white-space: pre-wrap; }

@media (max-width: 560px) {
  .global-search { width: calc(100vw - 20px); }
  .search-result { grid-template-columns: 64px minmax(0, 1fr); gap: 10px; padding-inline: 10px; }
  .search-result-heading, .search-result-trailing { gap: 8px; }
  .search-result-project { display: none; }
  .search-result-trailing { flex: none; max-width: none; }
}
</style>
