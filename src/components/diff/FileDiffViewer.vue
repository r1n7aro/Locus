<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { ChevronDown, ChevronUp } from "lucide";
import { diffSemanticTarget, diffTextForLarge, invalidateDiffCache, parseDiffRequestKey } from "../../services/diff";
import { gitExecute } from "../../services/git";
import { t } from "../../i18n";
import { useResizablePanel } from "../../composables/useResizablePanel";
import { highlightDiffHunk } from "./fileDiffText";
import type {
  FileDiffPayload,
  DiffHunk,
  DiffLine,
  TextDiff,
  SemanticTargetInspector,
  SemanticTargetSummary,
  SemanticTreeNode,
} from "../../types";
import UnityHierarchyPane from "./UnityHierarchyPane.vue";
import UnityInspectorPane from "./UnityInspectorPane.vue";
import BinaryPreviewHost from "./BinaryPreviewHost.vue";
import LucideIcon from "../icons/LucideIcon.vue";

const props = withDefaults(
  defineProps<{
    payload: FileDiffPayload;
    mode?: "unified" | "side-by-side";
    compact?: boolean;
    /** Line filter for full-code view: "all" = both red/green, "before" = old only, "after" = new only */
    filter?: "all" | "before" | "after";
    /** When true, the parent controls the tab switcher — hide the built-in tab bar */
    hideBuiltinTabs?: boolean;
    /** When true, the parent supplies the surrounding preview header */
    hideSemanticSummary?: boolean;
    /** When true, the parent controls text display mode actions — hide the built-in toolbar */
    hideTextDisplayControls?: boolean;
    /** Preferred initial tab when a fresh payload is mounted */
    initialTab?: "semantic" | "text";
  }>(),
  { mode: "unified", compact: false, filter: "all", hideBuiltinTabs: false, hideSemanticSummary: false, hideTextDisplayControls: false },
);

const emit = defineEmits<{
  lfsPulled: [];
}>();

const lfsPulling = ref(false);
const lfsPullError = ref<string | null>(null);

async function pullLfsObject() {
  const path = props.payload.filePath.replace(/\\/g, "/");
  lfsPulling.value = true;
  lfsPullError.value = null;
  try {
    const result = await gitExecute(`git lfs pull --include="${path}"`);
    if (result.exitCode !== 0) {
      lfsPullError.value = result.stderr.trim() || "git lfs pull failed";
      return;
    }
    invalidateDiffCache(props.payload.key);
    emit("lfsPulled");
  } catch (e: any) {
    lfsPullError.value = e?.message ?? String(e);
  } finally {
    lfsPulling.value = false;
  }
}

function resolveInitialTab(payload: FileDiffPayload): "text" | "semantic" {
  if (props.initialTab === "text" && (!!payload.text || payload.isLarge || !payload.semantic)) return "text";
  if (props.initialTab === "semantic" && !!payload.semantic) return "semantic";
  return payload.semantic ? "semantic" : "text";
}

const activeTab = ref<"text" | "semantic">(resolveInitialTab(props.payload));
const textDisplayMode = ref<"unified" | "side-by-side">(props.mode);
const selectedTargetId = ref<string | null>(null);
const includeUnchanged = ref(false);
const semanticLoading = ref(false);
const semanticError = ref<string | null>(null);
const activeInspector = ref<SemanticTargetInspector | null>(props.payload.semantic?.inspector ?? null);
const inspectorCache = ref(new Map<string, SemanticTargetInspector>());

/* ── On-demand text diff for large files ── */
const lazyText = ref<TextDiff | null>(null);
const lazyTextLoading = ref(false);
const lazyTextError = ref<string | null>(null);

async function loadTextDiff() {
  const request = parseDiffRequestKey(props.payload.key);
  if (!request) return;
  lazyTextLoading.value = true;
  lazyTextError.value = null;
  try {
    lazyText.value = await diffTextForLarge(request);
  } catch (e: any) {
    lazyTextError.value = e?.message ?? String(e);
  } finally {
    lazyTextLoading.value = false;
  }
}

/* ── Large-file text rendering optimization ── */
const CHUNK_SIZE = 500;
const HIGHLIGHT_LINE_LIMIT = 5000;
const textReady = ref(false);
const renderLimit = ref(CHUNK_SIZE);
const textScrollEl = ref<HTMLElement | null>(null);
const sceneLayoutRef = ref<HTMLElement | null>(null);
const {
  size: hierarchyWidth,
  isDragging: resizingHierarchy,
  onMouseDown: onHierarchyResizeMouseDown,
} = useResizablePanel(sceneLayoutRef, {
  storageKey: "locus:diff:scene-hierarchy-width",
  defaultSize: 280,
  minSize: 180,
  maxSize: (container) => Math.max(220, Math.min(520, container.clientWidth * 0.55)),
});

const effectiveText = computed(() => lazyText.value ?? props.payload.text);

const totalLineCount = computed(() =>
  effectiveText.value?.hunks.reduce((sum, h) => sum + h.lines.length, 0) ?? 0,
);

const visibleHunks = computed<{ hunk: DiffHunk; originalIndex: number }[]>(() => {
  if (!effectiveText.value) return [];
  let lineCount = 0;
  const result: { hunk: DiffHunk; originalIndex: number }[] = [];
  for (let i = 0; i < effectiveText.value.hunks.length; i++) {
    if (lineCount >= renderLimit.value) break;
    const hunk = effectiveText.value.hunks[i];
    const remaining = renderLimit.value - lineCount;
    if (hunk.lines.length <= remaining) {
      result.push({ hunk, originalIndex: i });
      lineCount += hunk.lines.length;
    } else {
      result.push({ hunk: { ...hunk, lines: hunk.lines.slice(0, remaining) }, originalIndex: i });
      lineCount += remaining;
    }
  }
  return result;
});

const hasMoreLines = computed(() => totalLineCount.value > renderLimit.value);

function onTextScroll(e: Event) {
  const el = e.target as HTMLElement;
  if (hasMoreLines.value && el.scrollTop + el.clientHeight >= el.scrollHeight - 300) {
    renderLimit.value += CHUNK_SIZE;
  }
  updateActiveChangeFromScroll();
}

function scheduleTextReady() {
  textReady.value = false;
  renderLimit.value = CHUNK_SIZE;
  nextTick(() => {
    requestAnimationFrame(() => {
      textReady.value = true;
    });
  });
}

const hasSemanticAndText = computed(
  () => !!props.payload.semantic && (!!effectiveText.value || props.payload.isLarge),
);

const hasTextDisplayModeControl = computed(
  () => !!effectiveText.value && (activeTab.value === "text" || !props.payload.semantic) && !props.compact,
);

const hasSemanticDetails = computed(() => {
  const semantic = props.payload.semantic;
  if (!semantic) return false;
  if (semantic.layout === "sceneHierarchyInspector") {
    return (semantic.tree?.length ?? 0) > 0;
  }
  return (semantic.targets?.length ?? 0) > 0 || !!semantic.inspector;
});

const largeFallbackText = computed(() =>
  props.compact
    ? "Too large"
    : props.payload.previewSummary?.[0] ?? "File too large for diff",
);

/** Strip fileID from labels like "StateSO (fileID:11400000)" */
function cleanLabel(label: string): string {
  return label.replace(/\s*\(fileID:\d+\)\s*/g, "").trim();
}

const semanticTargets = computed(() => props.payload.semantic?.targets ?? []);
const hasMultipleAssetTargets = computed(
  () => props.payload.semantic?.layout === "assetInspector" && semanticTargets.value.length > 1,
);

type ActiveAssetTarget = Pick<SemanticTargetSummary, "id" | "label" | "subtitle" | "path" | "changeKind" | "targetKind" | "scriptClass">;

const activeAssetTarget = computed<ActiveAssetTarget | null>(() => {
  const semantic = props.payload.semantic;
  if (!semantic || semantic.layout !== "assetInspector") return null;
  const preferredId = selectedTargetId.value ?? semantic.defaultTargetId ?? activeInspector.value?.targetId ?? null;
  if (preferredId) {
    const matched = semanticTargets.value.find((target) => target.id === preferredId);
    if (matched) return matched;
  }
  if (semanticTargets.value[0]) return semanticTargets.value[0];
  if (!activeInspector.value) return null;
  return {
    id: activeInspector.value.targetId,
    label: activeInspector.value.title,
    subtitle: activeInspector.value.subtitle,
    path: activeInspector.value.path,
    changeKind: "modified",
    scriptClass: semantic.scriptClassName,
  };
});

const semanticSummary = computed(() => {
  const semantic = props.payload.semantic;
  if (!semantic) return [];
  const parts: string[] = [];
  if (semantic.layout === "sceneHierarchyInspector") {
    if (semantic.summary.changedObjects) parts.push(t("diff.summary.objects", semantic.summary.changedObjects));
    if (semantic.summary.changedComponents) parts.push(t("diff.summary.components", semantic.summary.changedComponents));
  } else {
    if (semantic.summary.changedTargets) parts.push(t("diff.summary.targets", semantic.summary.changedTargets));
  }
  if (semantic.summary.changedFields) parts.push(t("diff.summary.fields", semantic.summary.changedFields));
  return parts;
});

const hierarchyColumnStyle = computed(() =>
  props.compact ? undefined : { width: `${hierarchyWidth.value}px` },
);

watch(
  () => props.payload,
  (payload) => {
    activeTab.value = resolveInitialTab(payload);
    textDisplayMode.value = props.mode;
    includeUnchanged.value = false;
    semanticError.value = null;
    semanticLoading.value = false;
    lazyText.value = null;
    lazyTextLoading.value = false;
    lazyTextError.value = null;
    inspectorCache.value = new Map();
    selectedTargetId.value =
      payload.semantic?.defaultTargetId ??
      payload.semantic?.targets?.[0]?.id ??
      null;
    activeInspector.value = payload.semantic?.inspector ?? null;
    if (payload.semantic?.inspector) {
      const key = `${payload.semantic.inspector.targetId}:0`;
      inspectorCache.value.set(key, payload.semantic.inspector);
    }
    // Gate text rendering so the loading indicator can paint first
    if (!payload.semantic || activeTab.value === "text") {
      scheduleTextReady();
    }
  },
  { immediate: true },
);

watch(
  () => props.mode,
  (mode) => {
    textDisplayMode.value = mode;
  },
);

watch(activeTab, (tab) => {
  if (tab === "text") {
    scheduleTextReady();
  }
});

watch(lazyText, (val) => {
  if (val) scheduleTextReady();
});

function cacheKey(targetId: string, showAll: boolean): string {
  return `${targetId}:${showAll ? "1" : "0"}`;
}

async function loadSemanticTarget(targetId: string, showAll = includeUnchanged.value) {
  if (!props.payload.semantic) return;
  const key = cacheKey(targetId, showAll);
  if (inspectorCache.value.has(key)) {
    activeInspector.value = inspectorCache.value.get(key)!;
    semanticError.value = null;
    return;
  }

  semanticLoading.value = true;
  semanticError.value = null;
  try {
    const inspector = await diffSemanticTarget({
      diffKey: props.payload.key,
      targetId,
      includeUnchanged: showAll,
    });
    inspectorCache.value.set(key, inspector);
    activeInspector.value = inspector;
  } catch (error: any) {
    console.error("[FileDiffViewer] failed to load semantic target:", {
      diffKey: props.payload.key,
      targetId,
      includeUnchanged: showAll,
      error,
    });
    semanticError.value = error?.message ?? String(error);
  } finally {
    semanticLoading.value = false;
  }
}

async function onSelectTarget(targetId: string) {
  selectedTargetId.value = targetId;
  await loadSemanticTarget(targetId);
}

async function toggleIncludeUnchanged() {
  includeUnchanged.value = !includeUnchanged.value;
  if (selectedTargetId.value) {
    await loadSemanticTarget(selectedTargetId.value, includeUnchanged.value);
  }
}

function toggleTextDisplayMode() {
  textDisplayMode.value = textDisplayMode.value === "unified" ? "side-by-side" : "unified";
}

function highlightedHunkForRender(hunk: DiffHunk): DiffHunk {
  return {
    ...hunk,
    lines: highlightDiffHunk(hunk, props.payload.filePath, totalLineCount.value > HIGHLIGHT_LINE_LIMIT),
  };
}

interface SideBySideRow {
  left: IndexedDiffLine | null;
  right: IndexedDiffLine | null;
  anchorKey: string | null;
}

interface IndexedDiffLine extends DiffLine {
  sourceLineIndex: number;
}

interface ChangeBlock {
  key: string;
  hunkIndex: number;
  startLineIndex: number;
  endLineIndex: number;
  renderLineOffset: number;
  lineLabel: string;
}

interface ScrollChangeCandidate {
  key: string;
  index: number;
  line: string;
  top: number;
  bottom: number;
  distance: number;
}

const fullContextTextMode = computed(() => Boolean(parseDiffRequestKey(props.payload.key)?.fullContext));

const isTextVisible = computed(() =>
  Boolean(effectiveText.value) && (activeTab.value === "text" || !props.payload.semantic),
);

const changeBlocks = computed<ChangeBlock[]>(() => {
  const text = effectiveText.value;
  if (!text) return [];
  const blocks: ChangeBlock[] = [];
  let lineOffset = 0;
  for (let hunkIndex = 0; hunkIndex < text.hunks.length; hunkIndex += 1) {
    const hunk = text.hunks[hunkIndex];
    let lineIndex = 0;
    while (lineIndex < hunk.lines.length) {
      if (hunk.lines[lineIndex].kind === "context") {
        lineIndex += 1;
        continue;
      }
      const startLineIndex = lineIndex;
      let endLineIndex = lineIndex;
      while (endLineIndex + 1 < hunk.lines.length && hunk.lines[endLineIndex + 1].kind !== "context") {
        endLineIndex += 1;
      }
      const firstChangedLine = hunk.lines[startLineIndex];
      const lineNo = firstChangedLine.newLineNo ?? firstChangedLine.oldLineNo ?? hunk.newStart ?? hunk.oldStart;
      blocks.push({
        key: `${hunkIndex}-${startLineIndex}`,
        hunkIndex,
        startLineIndex,
        endLineIndex,
        renderLineOffset: lineOffset + startLineIndex,
        lineLabel: String(lineNo),
      });
      lineIndex = endLineIndex + 1;
    }
    lineOffset += hunk.lines.length;
  }
  return blocks;
});

const changeBlockKeyByPosition = computed(() => {
  const map = new Map<string, string>();
  for (const block of changeBlocks.value) {
    map.set(`${block.hunkIndex}:${block.startLineIndex}`, block.key);
  }
  return map;
});

const activeChangeIndex = ref(0);
let suppressScrollActiveUntil = 0;
const PROGRAMMATIC_SCROLL_SUPPRESS_MS = 650;

const activeChange = computed(() => changeBlocks.value[activeChangeIndex.value] ?? null);

const showChangeNavigator = computed(() =>
  !props.compact
  && fullContextTextMode.value
  && isTextVisible.value
  && textReady.value
  && changeBlocks.value.length > 0,
);

watch(changeBlocks, (blocks) => {
  activeChangeIndex.value = Math.min(activeChangeIndex.value, Math.max(0, blocks.length - 1));
});

watch(showChangeNavigator, (show) => {
  if (!show) return;
  nextTick(() => updateActiveChangeFromScroll());
});

function changeAnchorKey(hunkIndex: number, lineIndex: number): string | null {
  return changeBlockKeyByPosition.value.get(`${hunkIndex}:${lineIndex}`) ?? null;
}

function isActiveChangeAnchor(hunkIndex: number, lineIndex: number): boolean {
  const key = changeAnchorKey(hunkIndex, lineIndex);
  return !!key && key === activeChange.value?.key;
}

function changeAnchorSelector(key: string): string {
  return `[data-change-anchor="${key}"]`;
}

async function ensureChangeRendered(block: ChangeBlock) {
  if (block.renderLineOffset < renderLimit.value) return;
  const nextLimit = Math.min(
    totalLineCount.value,
    Math.ceil((block.renderLineOffset + 80) / CHUNK_SIZE) * CHUNK_SIZE,
  );
  renderLimit.value = Math.max(renderLimit.value, nextLimit);
  await nextTick();
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
}

function updateActiveChangeFromScroll() {
  if (!showChangeNavigator.value || !textScrollEl.value) return;
  const now = Date.now();
  if (now < suppressScrollActiveUntil) {
    return;
  }
  const container = textScrollEl.value;
  const containerRect = container.getBoundingClientRect();
  const centerY = containerRect.top + containerRect.height / 2;
  const anchors = Array.from(container.querySelectorAll<HTMLElement>("[data-change-anchor]"));
  const candidates = anchors
    .flatMap((anchor): ScrollChangeCandidate[] => {
      const key = anchor.dataset.changeAnchor;
      if (!key) return [];
      const index = changeBlocks.value.findIndex((block) => block.key === key);
      if (index < 0) return [];
      const rect = anchor.getBoundingClientRect();
      const anchorCenterY = rect.top + rect.height / 2;
      return [{
        key,
        index,
        line: changeBlocks.value[index]?.lineLabel ?? "",
        top: Math.round(rect.top - containerRect.top),
        bottom: Math.round(rect.bottom - containerRect.top),
        distance: Math.round(Math.abs(anchorCenterY - centerY)),
      }];
    })
    .filter((candidate) => candidate.bottom >= 0 && candidate.top <= containerRect.height)
    .sort((a, b) => a.distance - b.distance);

  const nextIndex = candidates[0]?.index ?? activeChangeIndex.value;
  const clampedIndex = Math.min(Math.max(nextIndex, 0), Math.max(0, changeBlocks.value.length - 1));
  activeChangeIndex.value = clampedIndex;
}

async function navigateChange(direction: -1 | 1) {
  if (!showChangeNavigator.value || changeBlocks.value.length === 0) {
    return;
  }
  const previousIndex = activeChangeIndex.value;
  const nextIndex = Math.min(
    Math.max(activeChangeIndex.value + direction, 0),
    changeBlocks.value.length - 1,
  );
  if (nextIndex === previousIndex) {
    return;
  }
  activeChangeIndex.value = nextIndex;
  const block = changeBlocks.value[nextIndex];
  await ensureChangeRendered(block);
  const anchor = textScrollEl.value?.querySelector<HTMLElement>(changeAnchorSelector(block.key));
  if (!anchor) return;
  suppressScrollActiveUntil = Date.now() + PROGRAMMATIC_SCROLL_SUPPRESS_MS;
  anchor.scrollIntoView({ block: "center", behavior: "smooth" });
  window.setTimeout(() => {
    suppressScrollActiveUntil = 0;
    updateActiveChangeFromScroll();
  }, 180);
}

function goPreviousChange() {
  void navigateChange(-1);
}

function goNextChange() {
  void navigateChange(1);
}

function isTextInputTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName.toLowerCase();
  return tag === "input" || tag === "textarea" || tag === "select";
}

function onDocumentKeydown(event: KeyboardEvent) {
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
  if (!showChangeNavigator.value || event.defaultPrevented || isTextInputTarget(event.target)) {
    return;
  }
  if (event.key === "ArrowDown") {
    event.preventDefault();
    void navigateChange(1);
  } else if (event.key === "ArrowUp") {
    event.preventDefault();
    void navigateChange(-1);
  }
}

function indexedLine(line: DiffLine, sourceLineIndex: number): IndexedDiffLine {
  return { ...line, sourceLineIndex };
}

function alignHunk(hunk: DiffHunk, hunkIndex: number): SideBySideRow[] {
  const rows: SideBySideRow[] = [];
  let index = 0;

  while (index < hunk.lines.length) {
    const line = hunk.lines[index];
    if (line.kind === "context") {
      const indexed = indexedLine(line, index);
      rows.push({ left: indexed, right: indexed, anchorKey: null });
      index += 1;
      continue;
    }
    if (line.kind === "delete") {
      const deletes: IndexedDiffLine[] = [];
      while (index < hunk.lines.length && hunk.lines[index].kind === "delete") {
        deletes.push(indexedLine(hunk.lines[index], index));
        index += 1;
      }
      const adds: IndexedDiffLine[] = [];
      while (index < hunk.lines.length && hunk.lines[index].kind === "add") {
        adds.push(indexedLine(hunk.lines[index], index));
        index += 1;
      }
      const rowCount = Math.max(deletes.length, adds.length);
      for (let i = 0; i < rowCount; i += 1) {
        rows.push({
          left: deletes[i] ?? null,
          right: adds[i] ?? null,
          anchorKey: i === 0
            ? changeAnchorKey(hunkIndex, deletes[0]?.sourceLineIndex ?? adds[0]?.sourceLineIndex ?? -1)
            : null,
        });
      }
      continue;
    }
    rows.push({
      left: null,
      right: indexedLine(line, index),
      anchorKey: changeAnchorKey(hunkIndex, index),
    });
    index += 1;
  }

  return rows;
}

function filterLines(lines: DiffLine[]): DiffLine[] {
  if (props.filter === "all") return lines;
  if (props.filter === "before") return lines.filter((l) => l.kind !== "add");
  return lines.filter((l) => l.kind !== "delete"); // "after"
}

function filterRows(rows: SideBySideRow[]): SideBySideRow[] {
  if (props.filter === "all") return rows;
  if (props.filter === "before") {
    return rows
      .filter((r) => r.left !== null)
      .map((r) => ({ left: r.left, right: null, anchorKey: r.anchorKey }));
  }
  return rows
    .filter((r) => r.right !== null)
    .map((r) => ({ left: null, right: r.right, anchorKey: r.anchorKey }));
}

function treeNodes(): SemanticTreeNode[] {
  return props.payload.semantic?.tree ?? [];
}

function formatLfsSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

defineExpose({
  activeTab,
  hasSemanticAndText,
  hasTextDisplayModeControl,
  textDisplayMode,
  toggleTextDisplayMode,
});

onMounted(() => {
  document.addEventListener("keydown", onDocumentKeydown);
});

onUnmounted(() => {
  document.removeEventListener("keydown", onDocumentKeydown);
});
</script>

<template>
  <div class="diff-viewer" :class="{ compact }" @scroll="onTextScroll">
    <div v-if="payload.isBinary" class="diff-binary-shell">
      <div v-if="payload.binaryPreview" class="diff-binary-preview">
        <BinaryPreviewHost
          :preview="payload.binaryPreview"
          :compact="compact"
          :diff-key="payload.key"
        />
      </div>
      <div v-else class="diff-fallback">Binary file, no text preview</div>
    </div>
    <div v-else-if="payload.contentState?.type === 'lfsNotFetched'" class="diff-fallback lfs-fallback">
      <p>Git LFS file ({{ formatLfsSize(payload.contentState.size) }})</p>
      <button
        class="lfs-pull-btn"
        :disabled="lfsPulling"
        @click="pullLfsObject"
      >
        {{ lfsPulling ? 'Pulling...' : 'Pull LFS Object' }}
      </button>
      <p v-if="lfsPullError" class="lfs-error">{{ lfsPullError }}</p>
    </div>
    <div v-else-if="payload.isLarge && !payload.semantic && !effectiveText" class="diff-fallback" :class="{ compact }">
      <p>{{ largeFallbackText }}</p>
      <button v-if="!compact" class="lfs-pull-btn" :disabled="lazyTextLoading" @click="loadTextDiff">
        {{ lazyTextLoading ? 'Computing...' : 'Load text diff' }}
      </button>
      <p v-if="!compact && lazyTextError" class="lfs-error">{{ lazyTextError }}</p>
    </div>
    <template v-else>
      <div v-if="hasSemanticAndText && !hideBuiltinTabs" class="diff-tabs">
        <button class="diff-tab" :class="{ active: activeTab === 'semantic' }" @click="activeTab = 'semantic'">
          {{ t("diff.tabs.semantic") }}
        </button>
        <button class="diff-tab" :class="{ active: activeTab === 'text' }" @click="activeTab = 'text'">
          {{ t("diff.tabs.text") }}
        </button>
      </div>

      <div v-if="payload.semantic && activeTab === 'semantic'" class="semantic-view">
        <div v-if="!hideSemanticSummary" class="semantic-summary">
          <template v-if="activeAssetTarget && !hasMultipleAssetTargets">
            <span class="summary-asset-name">{{ cleanLabel(activeAssetTarget.label) }}</span>
            <span v-if="payload.semantic?.summary.changedFields" class="summary-text">{{ t('diff.summary.fields', payload.semantic.summary.changedFields) }}</span>
          </template>
          <span v-else class="summary-text">{{ semanticSummary.join(" · ") }}</span>
          <span v-if="!compact" class="summary-spacer"></span>
          <button v-if="!compact" class="summary-toggle-btn" :class="{ active: includeUnchanged }" @click="toggleIncludeUnchanged">
            {{ t('diff.fields.showUnchanged') }}
          </button>
        </div>

        <div v-if="!hasSemanticDetails" class="semantic-preview">
          Semantic summary is available in full diff view.
        </div>

        <template v-else-if="payload.semantic.layout === 'sceneHierarchyInspector'">
          <div
            ref="sceneLayoutRef"
            class="semantic-layout scene-layout"
            :class="{ 'resizing-hierarchy': resizingHierarchy }"
          >
            <div class="hierarchy-column" :style="hierarchyColumnStyle">
              <UnityHierarchyPane
                :nodes="treeNodes()"
                :selected-id="selectedTargetId"
                :hide-title="compact"
                :auto-collapse-when-overflow="compact"
                @select="onSelectTarget"
              />
            </div>
            <div
              v-if="!compact"
              class="hierarchy-resize-handle"
              role="separator"
              aria-orientation="vertical"
              @mousedown="onHierarchyResizeMouseDown"
            />
            <div class="inspector-column">
              <UnityInspectorPane
                :inspector="activeInspector"
                :loading="semanticLoading"
                :error="semanticError"
                :include-unchanged="includeUnchanged"
                :hide-toolbar="true"
                @toggle-unchanged="toggleIncludeUnchanged"
              />
            </div>
          </div>
        </template>

        <template v-else-if="hasMultipleAssetTargets">
          <div class="semantic-layout asset-sidebar-layout">
            <div class="asset-sidebar">
              <div class="asset-sidebar-title">{{ t("diff.summary.targets", payload.semantic.targets?.length ?? 0) }}</div>
              <div
                v-for="target in payload.semantic.targets"
                :key="target.id"
                class="asset-sidebar-row"
                :class="[
                  target.changeKind,
                  { selected: target.id === selectedTargetId },
                ]"
                @click="onSelectTarget(target.id)"
              >
                <span class="row-change-bar" :class="target.changeKind" />
                <span class="asset-sidebar-label">{{ cleanLabel(target.label) }}</span>
                <span v-if="target.subtitle && !target.subtitle.match(/^\s*\(?fileID:\d+\)?\s*$/)" class="asset-sidebar-subtitle">{{ cleanLabel(target.subtitle) }}</span>
                <span class="asset-sidebar-badge" :class="target.changeKind">
                  {{ target.changeKind === 'added' ? 'A' : target.changeKind === 'removed' ? 'D' : 'M' }}
                </span>
              </div>
            </div>
            <div class="inspector-column">
              <UnityInspectorPane
                :inspector="activeInspector"
                :loading="semanticLoading"
                :error="semanticError"
                :include-unchanged="includeUnchanged"
                :hide-toolbar="true"
                @toggle-unchanged="toggleIncludeUnchanged"
              />
            </div>
          </div>
        </template>

        <template v-else>
          <div class="semantic-layout asset-single-layout">
            <div class="inspector-column">
              <UnityInspectorPane
                :inspector="activeInspector"
                :loading="semanticLoading"
                :error="semanticError"
                :include-unchanged="includeUnchanged"
                :hide-toolbar="true"
                @toggle-unchanged="toggleIncludeUnchanged"
              />
            </div>
          </div>
        </template>
      </div>

      <!-- On-demand text diff loading for large files -->
      <div v-if="!effectiveText && (activeTab === 'text' || !payload.semantic)" class="diff-fallback" :class="{ compact }">
        <p>{{ largeFallbackText }}</p>
        <button v-if="!compact" class="lfs-pull-btn" :disabled="lazyTextLoading" @click="loadTextDiff">
          {{ lazyTextLoading ? 'Computing...' : 'Load text diff' }}
        </button>
        <p v-if="!compact && lazyTextError" class="lfs-error">{{ lazyTextError }}</p>
      </div>
      <div v-if="hasTextDisplayModeControl && !hideTextDisplayControls" class="diff-view-controls">
        <span class="summary-spacer"></span>
        <button class="summary-toggle-btn" :class="{ active: textDisplayMode === 'side-by-side' }" @click="toggleTextDisplayMode">
          {{ t('diff.mode.sideBySide') }}
        </button>
      </div>
      <div
        v-if="effectiveText && (activeTab === 'text' || !payload.semantic)"
        ref="textScrollEl"
        class="diff-text"
        :class="[textDisplayMode, { 'has-change-nav': showChangeNavigator }]"
        @scroll="onTextScroll"
      >
        <!-- Loading indicator while preparing large text -->
        <div v-if="!textReady" class="diff-loading">Loading…</div>
        <template v-else>
          <template v-for="{ hunk, originalIndex } in visibleHunks" :key="originalIndex">
            <div v-if="originalIndex > 0 && !compact" class="diff-hunk-separator">
              <span class="diff-hunk-header">{{ hunk.header }}</span>
            </div>

            <template v-if="textDisplayMode === 'unified'">
              <div
                v-for="(line, lineIndex) in filterLines(highlightedHunkForRender(hunk).lines)"
                :key="`${originalIndex}-${lineIndex}`"
                class="diff-line"
                :class="[line.kind, { 'change-anchor-active': isActiveChangeAnchor(originalIndex, lineIndex) }]"
                :data-change-anchor="changeAnchorKey(originalIndex, lineIndex) || undefined"
              >
                <span class="diff-ln">{{ filter === 'before' ? (line.oldLineNo ?? "") : filter === 'after' ? (line.newLineNo ?? "") : (line.kind === 'delete' ? (line.oldLineNo ?? "") : (line.newLineNo ?? "")) }}</span>
                <span class="diff-indicator">
                  {{ line.kind === "add" ? "+" : line.kind === "delete" ? "-" : " " }}
                </span>
                <span class="diff-content" v-html="line.content"></span>
              </div>
            </template>

            <template v-else>
              <div
                v-for="(row, rowIndex) in filterRows(alignHunk(highlightedHunkForRender(hunk), originalIndex))"
                :key="`${originalIndex}-${rowIndex}`"
                class="diff-sbs-row"
                :class="{ 'change-anchor-active': row.anchorKey === activeChange?.key }"
                :data-change-anchor="row.anchorKey || undefined"
              >
                <div class="diff-sbs-cell left" :class="row.left?.kind ?? 'empty'">
                  <span class="diff-ln">{{ row.left?.oldLineNo ?? "" }}</span>
                  <span class="diff-content" v-html="row.left?.content ?? '&nbsp;'"></span>
                </div>
                <div class="diff-sbs-cell right" :class="row.right?.kind ?? 'empty'">
                  <span class="diff-ln">{{ row.right?.newLineNo ?? "" }}</span>
                  <span class="diff-content" v-html="row.right?.content ?? '&nbsp;'"></span>
                </div>
              </div>
            </template>
          </template>
          <div v-if="hasMoreLines" class="diff-load-more">
            Showing {{ renderLimit }} of {{ totalLineCount }} lines — scroll down to load more
          </div>
        </template>
      </div>
      <div v-if="showChangeNavigator" class="diff-change-nav" role="toolbar" :aria-label="t('diff.nav.changeNavigation')">
        <button
          type="button"
          class="diff-change-nav-btn"
          :title="t('diff.nav.previousChange')"
          :disabled="activeChangeIndex <= 0"
          @click="goPreviousChange"
        >
          <LucideIcon :icon="ChevronUp" :size="15" />
        </button>
        <span class="diff-change-nav-count">
          {{ activeChangeIndex + 1 }} / {{ changeBlocks.length }}
          <span v-if="activeChange" class="diff-change-nav-line">L{{ activeChange.lineLabel }}</span>
        </span>
        <button
          type="button"
          class="diff-change-nav-btn"
          :title="t('diff.nav.nextChange')"
          :disabled="activeChangeIndex >= changeBlocks.length - 1"
          @click="goNextChange"
        >
          <LucideIcon :icon="ChevronDown" :size="15" />
        </button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.diff-viewer {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100%;
  font-family: var(--font-mono-editor);
  font-size: 13px;
  line-height: 1.5;
  overflow: auto;
}

.diff-viewer.compact {
  font-size: 11px;
  line-height: 1.4;
  max-height: 220px;
}

.diff-fallback {
  padding: 16px;
  text-align: center;
  color: var(--text-secondary);
}

.diff-fallback.compact {
  padding: 28px 16px;
  font-size: 12px;
  text-transform: lowercase;
}

.diff-binary-shell {
  flex: 1;
  display: flex;
  min-height: 0;
  overflow: hidden;
}

.diff-binary-preview {
  flex: 1;
  display: flex;
  min-height: 0;
  overflow: hidden;
}

.lfs-fallback p {
  margin: 0 0 8px;
}
.lfs-pull-btn {
  padding: 4px 12px;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: var(--bg-secondary);
  color: var(--text-primary);
  cursor: pointer;
  font-size: 12px;
}
.lfs-pull-btn:hover:not(:disabled) {
  background: var(--bg-hover);
}
.lfs-pull-btn:disabled {
  opacity: 0.6;
  cursor: default;
}
.lfs-error {
  color: var(--danger);
  font-size: 12px;
  margin-top: 6px;
}

.diff-tabs {
  display: flex;
  border-bottom: 1px solid var(--border-color);
}

.diff-tab {
  padding: 6px 16px;
  border: none;
  border-bottom: 2px solid transparent;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
}

.diff-tab.active {
  color: var(--accent-color);
  border-bottom-color: var(--accent-color);
}

.semantic-view {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
}

.semantic-summary,
.diff-view-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 14px;
  border-bottom: 1px solid var(--border-color);
}

.diff-viewer.compact .semantic-summary {
  padding: 6px 10px;
}

.summary-asset-name {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.summary-asset-badge {
  padding: 0px 5px;
  border-radius: 3px;
  font-size: 10px;
  font-weight: 700;
  flex-shrink: 0;
}

.summary-asset-badge.added {
  color: var(--git-status-added);
  background: color-mix(in srgb, var(--git-status-added) 16%, var(--bg-color));
}

.summary-asset-badge.removed {
  color: var(--git-status-deleted);
  background: color-mix(in srgb, var(--git-status-deleted) 16%, var(--bg-color));
}

.summary-asset-badge.modified {
  color: var(--git-status-modified);
  background: color-mix(in srgb, var(--git-status-modified) 16%, var(--bg-color));
}

.summary-asset-script {
  font-size: 11px;
  color: var(--text-secondary);
  flex-shrink: 0;
}

.summary-sep {
  width: 1px;
  height: 12px;
  background: var(--border-color);
  flex-shrink: 0;
}

.summary-text {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
}

.summary-spacer {
  flex: 1;
}

.summary-toggle-btn {
  padding: 2px 8px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 11px;
  white-space: nowrap;
  flex-shrink: 0;
  transition: all 0.15s;
}

.summary-toggle-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.summary-toggle-btn.active {
  background: var(--accent-color);
  color: #fff;
  border-color: var(--accent-color);
}

.semantic-preview {
  padding: 16px;
  color: var(--text-secondary);
}

.semantic-layout {
  display: flex;
  min-height: 0;
  height: 100%;
}

.semantic-layout.resizing-hierarchy {
  cursor: col-resize;
}

.diff-viewer.compact .semantic-layout {
  min-height: 170px;
}

.scene-layout .hierarchy-column {
  width: 280px;
  min-width: 240px;
  max-width: 520px;
  flex-shrink: 0;
  overflow: hidden;
}

.diff-viewer.compact .scene-layout .hierarchy-column {
  width: 38%;
  min-width: 150px;
  max-width: 360px;
}

.hierarchy-resize-handle {
  position: relative;
  width: 5px;
  flex-shrink: 0;
  cursor: col-resize;
  background: color-mix(in srgb, var(--border-color) 70%, transparent);
}

.hierarchy-resize-handle::before {
  content: "";
  position: absolute;
  inset: 0 2px;
  background: transparent;
  transition: background 0.15s;
}

.hierarchy-resize-handle:hover::before,
.scene-layout.resizing-hierarchy .hierarchy-resize-handle::before {
  background: var(--accent-color);
}

.scene-layout .inspector-column {
  flex: 1;
  min-width: 0;
}

/* Asset sidebar layout — mirrors scene-layout */
.asset-sidebar-layout {
  flex: 1;
  min-width: 0;
}

.asset-sidebar-layout > .asset-sidebar {
  width: 32%;
  min-width: 200px;
  max-width: 320px;
}

.diff-viewer.compact .asset-sidebar-layout > .asset-sidebar {
  width: 38%;
  min-width: 150px;
}

.asset-sidebar-layout > .inspector-column {
  flex: 1;
  min-width: 0;
}

.asset-sidebar {
  height: 100%;
  overflow: auto;
  border-right: 1px solid var(--border-color);
  background: var(--bg-secondary);
  font-family: var(--font-ui);
}

.asset-sidebar-title {
  padding: 8px 12px;
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border-color);
}

.asset-sidebar-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 28px;
  padding: 4px 10px;
  cursor: pointer;
  border-bottom: 1px solid var(--border-color);
  font-size: 12.5px;
}

.asset-sidebar-row:hover {
  background: var(--bg-hover);
}

.asset-sidebar-row.selected {
  background: color-mix(in srgb, var(--git-focus) 12%, var(--bg-color));
  box-shadow: inset 3px 0 0 var(--git-focus);
}

.asset-sidebar-row .row-change-bar {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
}

.asset-sidebar-row .row-change-bar.added { background: var(--git-status-added); }
.asset-sidebar-row .row-change-bar.removed { background: var(--git-status-deleted); }
.asset-sidebar-row .row-change-bar.modified { background: var(--git-status-modified); }

.asset-sidebar-label {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 600;
  color: var(--text-color);
}

.asset-sidebar-subtitle {
  font-size: 10px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 80px;
}

.asset-sidebar-badge {
  flex-shrink: 0;
  width: 16px;
  height: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 3px;
  font-size: 10px;
  font-weight: 700;
}

.asset-sidebar-badge.added {
  color: var(--git-status-added);
  background: color-mix(in srgb, var(--git-status-added) 16%, var(--bg-color));
}

.asset-sidebar-badge.removed {
  color: var(--git-status-deleted);
  background: color-mix(in srgb, var(--git-status-deleted) 16%, var(--bg-color));
}

.asset-sidebar-badge.modified {
  color: var(--git-status-modified);
  background: color-mix(in srgb, var(--git-status-modified) 16%, var(--bg-color));
}

.asset-single-layout {
  flex-direction: column;
}

.asset-single-layout > .inspector-column {
  flex: 1;
  min-height: 0;
}


.diff-text {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.diff-text.has-change-nav {
  padding-bottom: 52px;
}

.diff-line {
  display: grid;
  grid-template-columns: 52px 18px minmax(0, 1fr);
  gap: 8px;
  padding: 2px 12px;
  white-space: pre;
}

.diff-line.add {
  background: color-mix(in srgb, var(--git-status-added) 10%, var(--bg-color));
}

.diff-line.delete {
  background: color-mix(in srgb, var(--git-status-deleted) 10%, var(--bg-color));
}

.diff-line.change-anchor-active,
.diff-sbs-row.change-anchor-active .diff-sbs-cell {
  box-shadow: inset 2px 0 0 var(--accent-color);
}

.diff-ln {
  color: var(--text-secondary);
  text-align: right;
}

.diff-indicator {
  color: var(--text-secondary);
}

.diff-content {
  overflow: hidden;
}

.diff-hunk-separator {
  padding: 8px 12px;
  border-top: 1px solid var(--border-color);
  border-bottom: 1px solid var(--border-color);
  color: var(--text-secondary);
  background: var(--bg-secondary);
}

.diff-sbs-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
}

.diff-sbs-cell {
  display: grid;
  grid-template-columns: 52px minmax(0, 1fr);
  gap: 8px;
  padding: 2px 12px;
}

.diff-sbs-cell.add {
  background: color-mix(in srgb, var(--git-status-added) 10%, var(--bg-color));
}

.diff-sbs-cell.delete {
  background: color-mix(in srgb, var(--git-status-deleted) 10%, var(--bg-color));
}

.diff-sbs-cell.empty {
  opacity: 0.5;
}

.diff-loading {
  padding: 32px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.diff-load-more {
  padding: 12px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 11px;
  border-top: 1px solid var(--border-color);
}

.diff-change-nav {
  position: absolute;
  left: 50%;
  bottom: 14px;
  z-index: 12;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-height: 34px;
  padding: 4px 6px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--sidebar-bg) 6%);
  color: var(--text-color);
  box-shadow: 0 8px 24px color-mix(in srgb, var(--text-color) 18%, transparent);
  transform: translateX(-50%);
}

.diff-change-nav-btn {
  width: 26px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.diff-change-nav-btn:hover:not(:disabled),
.diff-change-nav-btn:focus-visible:not(:disabled) {
  background: var(--hover-bg);
  border-color: var(--border-color);
  color: var(--text-color);
  outline: none;
}

.diff-change-nav-btn:disabled {
  opacity: 0.45;
  cursor: default;
}

.diff-change-nav-count {
  min-width: 72px;
  display: inline-flex;
  align-items: baseline;
  justify-content: center;
  gap: 6px;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  line-height: 1;
  white-space: nowrap;
}

.diff-change-nav-line {
  color: var(--text-secondary);
  font-size: 11px;
}
</style>
