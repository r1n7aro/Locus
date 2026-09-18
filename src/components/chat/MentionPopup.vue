
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import LucideIcon from "../icons/LucideIcon.vue";
import type { LocusFileDropRef } from "../../services/unity";
import FileTreeList from "../explorer/FileTreeList.vue";
import { mentionResultKey } from "./mentionSearchRanking";
import {
  unityAssetIconClassForPath,
  unityAssetIconNodeForPath,
} from "../icons/unityAssetIcons";

export interface MentionDisplayEntry {
  relPath: string;
  name: string;
  parentPath?: string;
  isDir: boolean;
  meta?: string;
  canNavigate?: boolean;
  isCurrentPath?: boolean;
  entryKind?: "asset" | "knowledge" | "sceneObject";
  localFile?: LocusFileDropRef;
}

const props = defineProps<{
  visible: boolean;
  mode: "search" | "browse";
  entries: MentionDisplayEntry[];
  selectedIndex: number;
  breadcrumbs: string[];
  query: string;
  loading: boolean;
  showEmpty: boolean;
}>();

const emit = defineEmits<{
  select: [entry: MentionDisplayEntry];
  openDir: [entry: MentionDisplayEntry];
  navigateTo: [level: number];
  navigateRoot: [];
  "update:selectedIndex": [index: number];
}>();

interface HighlightFragment {
  text: string;
  matched: boolean;
}

const popupRef = ref<HTMLElement | null>(null);
const listRef = ref<InstanceType<typeof FileTreeList> | null>(null);
const maxHeight = ref(520);
const rows = computed(() => props.entries.map((entry) => ({ key: mentionResultKey(entry), entry })));
const rowHeight = computed(() => props.entries.some((entry) => entry.meta || entry.parentPath) ? 48 : 36);
const terms = computed(() => highlightTerms(props.query));
const presentationCache = new WeakMap<MentionDisplayEntry, {
  query: string;
  nameText: string;
  pathText: string;
  name: HighlightFragment[];
  path: HighlightFragment[];
}>();
let pointerSelectionIndex: number | null = null;
let lastPointerPosition: { x: number; y: number } | null = null;

function highlightFromPointer(event: MouseEvent, index: number) {
  // Scrolling or inserting results under a stationary pointer must not change selection.
  if (lastPointerPosition?.x === event.clientX && lastPointerPosition.y === event.clientY) return;
  lastPointerPosition = { x: event.clientX, y: event.clientY };
  if (props.selectedIndex === index) return;
  pointerSelectionIndex = index;
  emit("update:selectedIndex", index);
}

function pageSize() {
  const height = listRef.value?.$el.clientHeight || maxHeight.value - 36;
  return Math.max(1, Math.floor(height / rowHeight.value));
}

defineExpose({ pageSize });

function highlightTerms(query: string): string[] {
  return Array.from(new Set(
    query
      .trim()
      .split(/[\s/\\._-]+/g)
      .map((part) => part.trim())
      .filter(Boolean)
      .sort((a, b) => b.length - a.length),
  ));
}

function buildFragments(text: string): HighlightFragment[] {
  if (!text) return [];
  if (terms.value.length === 0) return [{ text, matched: false }];

  const lowerText = text.toLocaleLowerCase();
  const ranges: Array<{ start: number; end: number }> = [];

  for (const term of terms.value) {
    const lowerTerm = term.toLocaleLowerCase();
    let startIndex = 0;
    while (startIndex < lowerText.length) {
      const matchIndex = lowerText.indexOf(lowerTerm, startIndex);
      if (matchIndex < 0) break;
      ranges.push({ start: matchIndex, end: matchIndex + lowerTerm.length });
      startIndex = matchIndex + lowerTerm.length;
    }
  }

  if (ranges.length === 0) return [{ text, matched: false }];

  ranges.sort((left, right) => left.start - right.start || left.end - right.end);
  const mergedRanges: Array<{ start: number; end: number }> = [];
  for (const range of ranges) {
    const previous = mergedRanges[mergedRanges.length - 1];
    if (!previous || range.start > previous.end) {
      mergedRanges.push({ ...range });
      continue;
    }
    previous.end = Math.max(previous.end, range.end);
  }

  const fragments: HighlightFragment[] = [];
  let cursor = 0;
  for (const range of mergedRanges) {
    if (range.start > cursor) {
      fragments.push({ text: text.slice(cursor, range.start), matched: false });
    }
    fragments.push({ text: text.slice(range.start, range.end), matched: true });
    cursor = range.end;
  }

  if (cursor < text.length) {
    fragments.push({ text: text.slice(cursor), matched: false });
  }

  return fragments;
}

function presentation(entry: MentionDisplayEntry) {
  const pathText = entry.meta || entry.parentPath || "";
  let cached = presentationCache.get(entry);
  if (!cached || cached.query !== props.query || cached.nameText !== entry.name || cached.pathText !== pathText) {
    cached = {
      query: props.query,
      nameText: entry.name,
      pathText,
      name: buildFragments(entry.name),
      path: buildFragments(pathText),
    };
    presentationCache.set(entry, cached);
  }
  return cached;
}

function iconNodeForEntry(entry: MentionDisplayEntry) {
  return unityAssetIconNodeForPath(entry.relPath, {
    isFolder: entry.isDir,
    isSceneObject: entry.entryKind === "sceneObject",
    fallbackKind: entry.entryKind === "knowledge" ? "asset" : "file",
  });
}

function iconClassForEntry(entry: MentionDisplayEntry) {
  return unityAssetIconClassForPath(entry.relPath, {
    isFolder: entry.isDir,
    isSceneObject: entry.entryKind === "sceneObject",
    fallbackKind: entry.entryKind === "knowledge" ? "asset" : "file",
  });
}

watch(popupRef, (popup, _, onCleanup) => {
  if (!popup) return;
  function updateAvailableHeight() {
    // The popup is anchored above the composer; its bottom is independent of its height.
    const bottom = popup!.getBoundingClientRect().bottom;
    if (bottom > 0) maxHeight.value = Math.max(0, Math.min(520, Math.floor(bottom - 8)));
  }
  updateAvailableHeight();
  const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(updateAvailableHeight);
  observer?.observe(popup);
  if (popup.parentElement) observer?.observe(popup.parentElement);
  window.addEventListener("resize", updateAvailableHeight);
  onCleanup(() => {
    observer?.disconnect();
    window.removeEventListener("resize", updateAvailableHeight);
  });
}, { flush: "post" });

watch(
  () => [props.visible, props.mode, props.query, props.breadcrumbs.join("/"), props.selectedIndex, props.entries.length],
  (current, previous) => {
    if (!props.visible) return;
    const contextChanged = current.slice(0, 4).some((value, index) => value !== previous?.[index]);
    const selectionChanged = current[4] !== previous?.[4];
    const pointerSelected = pointerSelectionIndex === props.selectedIndex;
    pointerSelectionIndex = null;
    if (contextChanged) lastPointerPosition = null;
    if (!contextChanged && !selectionChanged) return;
    // Mouse hover must not pull a partially visible row (and its neighbours) under the pointer.
    if (pointerSelected && !contextChanged) return;
    listRef.value?.scrollToIndex(props.selectedIndex, contextChanged ? { align: "center" } : undefined);
  },
  { immediate: true, flush: "post" },
);
</script>

<template>
  <div v-if="visible" ref="popupRef" class="mention-popup" :style="{ maxHeight: `${maxHeight}px` }">
    <div v-if="mode === 'browse'" class="mention-breadcrumb">
      <button
        type="button"
        class="mention-crumb"
        :class="{ active: breadcrumbs.length === 0 }"
        @mousedown.prevent
        @click="emit('navigateRoot')"
      >./</button>
      <template v-for="(part, idx) in breadcrumbs" :key="idx">
        <span class="mention-crumb-sep">/</span>
        <button
          type="button"
          class="mention-crumb"
          :class="{ active: idx === breadcrumbs.length - 1 }"
          @mousedown.prevent
          @click="emit('navigateTo', idx)"
        >{{ part }}</button>
      </template>
      <span v-if="loading && entries.length > 0" class="mention-loading-status">{{ t('chat.mention.loading') }}</span>
    </div>
    <div v-else class="mention-search-header">
      <span class="mention-search-label">{{ t('chat.mention.assetSearch') }}</span>
      <span v-if="loading && entries.length > 0" class="mention-loading-status">{{ t('chat.mention.loading') }}</span>
    </div>
    <div v-if="loading && entries.length === 0" class="mention-loading">{{ t('chat.mention.loading') }}</div>
    <div v-else-if="showEmpty" class="mention-empty">{{ t('chat.mention.noMatch') }}</div>
    <FileTreeList
      v-else
      ref="listRef"
      class="mention-results"
      role="listbox"
      :aria-label="t('chat.mention.assetSearch')"
      :aria-busy="loading"
      :items="rows"
      :row-height="rowHeight"
      :overscan="4"
    >
      <template #item="{ item, index: idx }">
        <div
          class="mention-item"
          :class="{ highlighted: idx === selectedIndex, 'is-current-path': item.entry.isCurrentPath }"
          :style="{ height: `${rowHeight}px` }"
          :aria-selected="idx === selectedIndex ? 'true' : 'false'"
          :aria-posinset="idx + 1"
          :aria-setsize="entries.length"
          role="option"
          @mousemove="highlightFromPointer($event, idx)"
        >
          <button
            type="button"
            class="mention-select"
            :title="item.entry.relPath"
            @mousedown.left.prevent="emit('select', item.entry)"
            @click="$event.detail === 0 && emit('select', item.entry)"
            @focus="emit('update:selectedIndex', idx)"
          >
            <LucideIcon
              class="mention-icon"
              :class="iconClassForEntry(item.entry)"
              :icon="iconNodeForEntry(item.entry)"
              :size="14"
            />
            <span class="mention-copy">
              <span class="mention-name">
                <span
                  v-for="(fragment, fragmentIdx) in presentation(item.entry).name"
                  :key="fragmentIdx"
                  class="mention-name-fragment"
                  :class="{ 'is-match': fragment.matched }"
                >{{ fragment.text }}</span>
              </span>
              <span
                v-if="item.entry.meta || item.entry.parentPath"
                class="mention-path"
                :title="item.entry.meta || item.entry.parentPath"
              >
                <span
                  v-for="(fragment, fragmentIdx) in presentation(item.entry).path"
                  :key="fragmentIdx"
                  class="mention-path-fragment"
                  :class="{ 'is-match': fragment.matched }"
                >{{ fragment.text }}</span>
              </span>
            </span>
          </button>
          <button
            v-if="item.entry.isDir && item.entry.canNavigate"
            type="button"
            class="mention-open"
            :title="t('chat.mention.openFolder')"
            :aria-label="t('chat.mention.openFolder')"
            @mousedown.left.prevent.stop="emit('openDir', item.entry)"
            @click.stop="$event.detail === 0 && emit('openDir', item.entry)"
          >&rsaquo;</button>
        </div>
      </template>
    </FileTreeList>
  </div>
</template>
