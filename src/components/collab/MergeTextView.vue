<script setup lang="ts">
import { ref, computed, watch } from "vue";
import type { MergeFileInfo, FileDiffPayload } from "../../types";
import { diffSingleFile } from "../../services/diff";
import { gitMergeApply } from "../../services/git";
import { normalizeAppError } from "../../services/errors";
import { useTextViewerZoom } from "../../composables/useTextViewerZoom";
import { t } from "../../i18n";
import { humanizeMergeSideLabel, sharedBaseLabel } from "./mergeUi";
import type { WorkspaceRef } from "../../services/project";

const props = defineProps<{
  workspaceRef?: WorkspaceRef | null;
  mergeInfo: MergeFileInfo;
  filePath: string;
  conflictOids: string;
  leftLabel?: string;
  rightLabel?: string;
  baseLabel?: string;
}>();

function captureWorkspaceRef(): WorkspaceRef {
  if (!props.workspaceRef) throw new Error("Workspace checkout is required.");
  return {
    checkoutId: props.workspaceRef.checkoutId,
    expectedGeneration: props.workspaceRef.expectedGeneration ?? undefined,
  };
}

function isCurrentWorkspaceRef(workspaceRef?: WorkspaceRef) {
  return (workspaceRef?.checkoutId ?? null) === (props.workspaceRef?.checkoutId ?? null)
    && (workspaceRef?.expectedGeneration ?? null)
      === (props.workspaceRef?.expectedGeneration ?? null);
}

const emit = defineEmits<{
  (e: "resolved"): void;
}>();

const resolvedText = ref(props.mergeInfo.workspaceText ?? "");
const saving = ref(false);
const error = ref<string | null>(null);
const { textViewerZoomStyle, handleTextViewerZoomWheel } = useTextViewerZoom();

const blockResolutions = ref<Map<number, "left" | "right" | "base">>(new Map());

const baseToLeftDiff = ref<FileDiffPayload | null>(null);
const baseToRightDiff = ref<FileDiffPayload | null>(null);
const leftDiffLoading = ref(false);
const rightDiffLoading = ref(false);
const diffsRequested = ref(false);
let diffLoadGeneration = 0;

const isManuallyEdited = computed(() => !props.mergeInfo.workspaceMatchesCanonical);
const displayLeftLabel = computed(() => humanizeMergeSideLabel(props.leftLabel, "left"));
const displayRightLabel = computed(() => humanizeMergeSideLabel(props.rightLabel, "right"));
const displayBaseLabel = computed(() => props.baseLabel ?? sharedBaseLabel());

const canUseBlockSelection = computed(() =>
  !props.mergeInfo.isBinary
  && !props.mergeInfo.isSubmodule
  && !isManuallyEdited.value
  && props.mergeInfo.conflictBlocks.length > 0
);

const allBlocksResolved = computed(() =>
  props.mergeInfo.conflictBlocks.every((_, i) => blockResolutions.value.has(i))
);

function loadDiffsIfNeeded() {
  if (diffsRequested.value) return;
  const workspaceRef = captureWorkspaceRef();
  const generation = ++diffLoadGeneration;
  diffsRequested.value = true;
  leftDiffLoading.value = true;
  rightDiffLoading.value = true;

  diffSingleFile({
    source: "gitConflictBaseToLeft",
    filePath: props.filePath,
    commitHash: props.conflictOids,
    detail: "preview",
  }, workspaceRef)
    .then((d) => {
      if (generation !== diffLoadGeneration || !isCurrentWorkspaceRef(workspaceRef)) return;
      baseToLeftDiff.value = d;
    })
    .catch((e) => {
      if (generation !== diffLoadGeneration || !isCurrentWorkspaceRef(workspaceRef)) return;
      console.error("[merge] left diff failed:", e);
    })
    .finally(() => {
      if (generation === diffLoadGeneration && isCurrentWorkspaceRef(workspaceRef)) {
        leftDiffLoading.value = false;
      }
    });

  diffSingleFile({
    source: "gitConflictBaseToRight",
    filePath: props.filePath,
    commitHash: props.conflictOids,
    detail: "preview",
  }, workspaceRef)
    .then((d) => {
      if (generation !== diffLoadGeneration || !isCurrentWorkspaceRef(workspaceRef)) return;
      baseToRightDiff.value = d;
    })
    .catch((e) => {
      if (generation !== diffLoadGeneration || !isCurrentWorkspaceRef(workspaceRef)) return;
      console.error("[merge] right diff failed:", e);
    })
    .finally(() => {
      if (generation === diffLoadGeneration && isCurrentWorkspaceRef(workspaceRef)) {
        rightDiffLoading.value = false;
      }
    });
}

watch(
  () => [
    props.conflictOids,
    props.workspaceRef?.checkoutId ?? null,
    props.workspaceRef?.expectedGeneration ?? null,
  ] as const,
  () => {
    diffLoadGeneration += 1;
    diffsRequested.value = false;
    baseToLeftDiff.value = null;
    baseToRightDiff.value = null;
    leftDiffLoading.value = false;
    rightDiffLoading.value = false;
    loadDiffsIfNeeded();
  },
  { immediate: true },
);

function selectBlock(blockIndex: number, side: "left" | "right" | "base") {
  const newMap = new Map(blockResolutions.value);
  newMap.set(blockIndex, side);
  blockResolutions.value = newMap;
  rebuildResolvedText();
}

function rebuildResolvedText() {
  if (!props.mergeInfo.workspaceText) return;

  const text = props.mergeInfo.workspaceText;
  const blocks = props.mergeInfo.conflictBlocks;
  const lines = text.split("\n");
  const result: string[] = [];
  let lastEnd = 0;

  for (const block of blocks) {
    const blockStartIdx = block.startLine - 1;
    for (let i = lastEnd; i < blockStartIdx; i++) {
      result.push(lines[i]);
    }

    const choice = blockResolutions.value.get(block.index);
    if (choice === "left") {
      if (block.leftContent) result.push(block.leftContent);
    } else if (choice === "right") {
      if (block.rightContent) result.push(block.rightContent);
    } else if (choice === "base") {
      if (block.baseContent) result.push(block.baseContent);
    } else {
      for (let i = blockStartIdx; i < block.endLine; i++) {
        result.push(lines[i]);
      }
    }

    lastEnd = block.endLine;
  }

  for (let i = lastEnd; i < lines.length; i++) {
    result.push(lines[i]);
  }

  resolvedText.value = result.join("\n");
}

async function applyResolution() {
  const workspaceRef = captureWorkspaceRef();
  saving.value = true;
  error.value = null;
  try {
    await gitMergeApply(
      props.filePath,
      { resolvedText: { text: resolvedText.value } },
      workspaceRef,
    );
    if (!isCurrentWorkspaceRef(workspaceRef)) return;
    emit("resolved");
  } catch (e) {
    if (!isCurrentWorkspaceRef(workspaceRef)) return;
    error.value = normalizeAppError(e).message;
  } finally {
    if (isCurrentWorkspaceRef(workspaceRef)) saving.value = false;
  }
}

function diffStatsText(diff: FileDiffPayload | null): string {
  if (!diff) return "";
  return `+${diff.stats.additions} -${diff.stats.deletions}`;
}

function handleMergeTextWheel(event: WheelEvent): void {
  if (!(event.target instanceof Element)) return;
  if (!event.target.closest(".merge-block-code, .merge-editor-textarea")) return;
  handleTextViewerZoomWheel(event);
}
</script>

<template>
  <div
    class="merge-text-view"
    :style="textViewerZoomStyle"
    @wheel="handleMergeTextWheel"
  >
    <div v-if="error" class="merge-action-error">{{ error }}</div>

    <div v-if="isManuallyEdited" class="merge-manual-edit-banner">
      {{ t("merge.banner.externalChanges") }}
    </div>

    <div v-if="canUseBlockSelection" class="merge-blocks">
      <div class="merge-blocks-header">
        <span>{{ t("merge.text.conflictBlocks", mergeInfo.conflictBlocks.length) }}</span>
        <span v-if="allBlocksResolved" class="merge-blocks-resolved">{{ t("merge.text.allBlocksResolved") }}</span>
      </div>
      <div
        v-for="block in mergeInfo.conflictBlocks"
        :key="block.index"
        class="merge-block-item"
        :class="{ resolved: blockResolutions.has(block.index) }"
      >
        <div class="merge-block-header">
          <span class="merge-block-label">{{ t("merge.text.blockLabel", block.index + 1, block.startLine, block.endLine) }}</span>
          <div class="merge-block-choices">
            <button
              class="merge-block-btn"
              :class="{ active: blockResolutions.get(block.index) === 'left' }"
              @click="selectBlock(block.index, 'left')"
            >{{ displayLeftLabel }}</button>
            <button
              class="merge-block-btn"
              :class="{ active: blockResolutions.get(block.index) === 'right' }"
              @click="selectBlock(block.index, 'right')"
            >{{ displayRightLabel }}</button>
            <button
              v-if="block.baseContent"
              class="merge-block-btn"
              :class="{ active: blockResolutions.get(block.index) === 'base' }"
              @click="selectBlock(block.index, 'base')"
            >{{ displayBaseLabel }}</button>
          </div>
        </div>
        <div class="merge-block-preview">
          <div class="merge-block-side">
            <div class="merge-block-side-label">{{ displayLeftLabel }}</div>
            <pre class="merge-block-code">{{ block.leftContent || "(empty)" }}</pre>
          </div>
          <div class="merge-block-side">
            <div class="merge-block-side-label">{{ displayRightLabel }}</div>
            <pre class="merge-block-code">{{ block.rightContent || "(empty)" }}</pre>
          </div>
        </div>
      </div>
    </div>

    <div class="merge-editor-section">
      <div class="merge-editor-header">
        <span>{{ t("merge.text.result") }}</span>
      </div>
      <textarea
        v-model="resolvedText"
        class="merge-editor-textarea"
        spellcheck="false"
      ></textarea>
    </div>

    <div class="merge-apply-row">
      <button
        class="merge-action-btn merge-continue-btn primary"
        :disabled="saving"
        @click="applyResolution"
      >
        {{ saving ? t("merge.actions.applyingText") : t("merge.actions.applyText") }}
      </button>
    </div>

    <div class="merge-diff-summaries">
      <div class="merge-diff-summary">
        <span class="merge-diff-summary-label">{{ t("merge.text.sharedBaseTo", displayLeftLabel) }}</span>
        <span v-if="leftDiffLoading" class="merge-diff-summary-loading">{{ t("merge.text.loading") }}</span>
        <span v-else-if="baseToLeftDiff" class="merge-diff-summary-stats">{{ diffStatsText(baseToLeftDiff) }}</span>
      </div>
      <div class="merge-diff-summary">
        <span class="merge-diff-summary-label">{{ t("merge.text.sharedBaseTo", displayRightLabel) }}</span>
        <span v-if="rightDiffLoading" class="merge-diff-summary-loading">{{ t("merge.text.loading") }}</span>
        <span v-else-if="baseToRightDiff" class="merge-diff-summary-stats">{{ diffStatsText(baseToRightDiff) }}</span>
      </div>
    </div>
  </div>
</template>
