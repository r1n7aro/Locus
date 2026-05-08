
<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import type { ComponentPublicInstance } from "vue";
import { t } from "../../i18n";
import LucideIcon from "../icons/LucideIcon.vue";
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
  entryKind?: "asset" | "knowledge";
}

const props = defineProps<{
  visible: boolean;
  mode: "search" | "browse";
  entries: MentionDisplayEntry[];
  selectedIndex: number;
  breadcrumbs: string[];
  query: string;
  loading: boolean;
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

const itemRefs = ref<HTMLElement[]>([]);
const popupRef = ref<HTMLElement | null>(null);

function resolveTemplateElement(
  element: Element | ComponentPublicInstance | null,
): Element | null {
  if (element instanceof Element) return element;
  if (element && "$el" in element && element.$el instanceof Element) {
    return element.$el;
  }
  return null;
}

function setItemRef(index: number, element: Element | ComponentPublicInstance | null) {
  const resolved = resolveTemplateElement(element);
  if (!(resolved instanceof HTMLElement)) return;
  itemRefs.value[index] = resolved;
}

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

function buildFragments(text: string, query: string): HighlightFragment[] {
  if (!text) return [];
  const terms = highlightTerms(query);
  if (terms.length === 0) return [{ text, matched: false }];

  const lowerText = text.toLocaleLowerCase();
  const ranges: Array<{ start: number; end: number }> = [];

  for (const term of terms) {
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

function iconNodeForEntry(entry: MentionDisplayEntry) {
  return unityAssetIconNodeForPath(entry.relPath, {
    isFolder: entry.isDir,
    fallbackKind: entry.entryKind === "knowledge" ? "asset" : "file",
  });
}

function iconClassForEntry(entry: MentionDisplayEntry) {
  return unityAssetIconClassForPath(entry.relPath, {
    isFolder: entry.isDir,
    fallbackKind: entry.entryKind === "knowledge" ? "asset" : "file",
  });
}

watch(
  () => [props.visible, props.selectedIndex, props.entries.length],
  async ([visible]) => {
    if (!visible) return;
    await nextTick();
    const popup = popupRef.value;
    const selected = itemRefs.value[props.selectedIndex];
    if (!popup || !selected) return;

    const itemTop = selected.offsetTop;
    const itemBottom = itemTop + selected.offsetHeight;
    const viewTop = popup.scrollTop;
    const viewBottom = viewTop + popup.clientHeight;

    if (itemTop < viewTop) {
      popup.scrollTop = itemTop;
      return;
    }

    if (itemBottom > viewBottom) {
      popup.scrollTop = itemBottom - popup.clientHeight;
    }
  },
);

watch(
  () => props.entries,
  () => {
    itemRefs.value = [];
  },
);
</script>

<template>
  <div v-if="visible" ref="popupRef" class="mention-popup" role="listbox">
    <div v-if="mode === 'browse'" class="mention-breadcrumb">
      <span
        class="mention-crumb"
        :class="{ active: breadcrumbs.length === 0 }"
        @mousedown.prevent="emit('navigateRoot')"
      >./</span>
      <template v-for="(part, idx) in breadcrumbs" :key="idx">
        <span class="mention-crumb-sep">/</span>
        <span
          class="mention-crumb"
          :class="{ active: idx === breadcrumbs.length - 1 }"
          @mousedown.prevent="emit('navigateTo', idx)"
        >{{ part }}</span>
      </template>
    </div>
    <div v-else class="mention-search-header">
      <span class="mention-search-label">{{ t('chat.mention.assetSearch') }}</span>
    </div>
    <div v-if="loading && entries.length === 0" class="mention-loading">{{ t('chat.mention.loading') }}</div>
    <div v-else-if="entries.length === 0" class="mention-empty">{{ t('chat.mention.noMatch') }}</div>
    <template v-else>
      <div
        v-for="(entry, idx) in entries"
        :key="entry.relPath"
        class="mention-item"
        :class="{ highlighted: idx === selectedIndex, 'is-current-path': entry.isCurrentPath }"
        :ref="(el) => setItemRef(idx, el)"
        :aria-selected="idx === selectedIndex ? 'true' : 'false'"
        role="option"
        @mouseenter="$emit('update:selectedIndex', idx)"
      >
        <button
          type="button"
          class="mention-select"
          @mousedown.prevent="emit('select', entry)"
        >
          <LucideIcon
            class="mention-icon"
            :class="iconClassForEntry(entry)"
            :icon="iconNodeForEntry(entry)"
            :size="14"
          />
          <span class="mention-copy">
            <span class="mention-name">
              <span
                v-for="(fragment, fragmentIdx) in buildFragments(entry.name, query)"
                :key="`${entry.relPath}-name-${fragmentIdx}`"
                class="mention-name-fragment"
                :class="{ 'is-match': fragment.matched }"
              >{{ fragment.text }}</span>
            </span>
            <span v-if="entry.meta || entry.parentPath" class="mention-path">
              <span
                v-for="(fragment, fragmentIdx) in buildFragments(entry.meta || entry.parentPath || '', query)"
                :key="`${entry.relPath}-path-${fragmentIdx}`"
                class="mention-path-fragment"
                :class="{ 'is-match': fragment.matched }"
              >{{ fragment.text }}</span>
            </span>
          </span>
        </button>
        <button
          v-if="entry.isDir && entry.canNavigate"
          type="button"
          class="mention-open"
          :title="t('chat.mention.openFolder')"
          :aria-label="t('chat.mention.openFolder')"
          @mousedown.prevent.stop="emit('openDir', entry)"
        >&rsaquo;</button>
      </div>
      <div v-if="loading" class="mention-loading mention-loading-inline">{{ t('chat.mention.loading') }}</div>
    </template>
  </div>
</template>
