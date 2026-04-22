<script setup lang="ts">
import { ref, computed } from "vue";
import type { MergeFileInfo, FileDiffPayload } from "../../types";
import { diffSingleFile } from "../../services/diff";
import { gitMergeApply } from "../../services/git";
import { normalizeAppError } from "../../services/errors";
import { t } from "../../i18n";
import { humanizeMergeSideLabel, sharedBaseLabel } from "./mergeUi";

const props = defineProps<{
  mergeInfo: MergeFileInfo;
  filePath: string;
  conflictOids: string;
  leftLabel?: string;
  rightLabel?: string;
  baseLabel?: string;
}>();

const emit = defineEmits<{
  (e: "resolved"): void;
}>();

const resolvedText = ref(props.mergeInfo.workspaceText ?? "");
const saving = ref(false);
const error = ref<string | null>(null);

const blockResolutions = ref<Map<number, "left" | "right" | "base">>(new Map());

const baseToLeftDiff = ref<FileDiffPayload | null>(null);
const baseToRightDiff = ref<FileDiffPayload | null>(null);
const leftDiffLoading = ref(false);
const rightDiffLoading = ref(false);
const diffsRequested = ref(false);

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
  diffsRequested.value = true;
  leftDiffLoading.value = true;
  rightDiffLoading.value = true;

  diffSingleFile({
    source: "gitConflictBaseToLeft",
    filePath: props.filePath,
    commitHash: props.conflictOids,
    detail: "preview",
  })
    .then((d) => { baseToLeftDiff.value = d; })
    .catch((e) => { console.error("[merge] left diff failed:", e); })
    .finally(() => { leftDiffLoading.value = false; });

  diffSingleFile({
    source: "gitConflictBaseToRight",
    filePath: props.filePath,
    commitHash: props.conflictOids,
    detail: "preview",
  })
    .then((d) => { baseToRightDiff.value = d; })
    .catch((e) => { console.error("[merge] right diff failed:", e); })
    .finally(() => { rightDiffLoading.value = false; });
}

loadDiffsIfNeeded();

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
  saving.value = true;
  error.value = null;
  try {
    await gitMergeApply(props.filePath, { resolvedText: { text: resolvedText.value } });
    emit("resolved");
  } catch (e) {
    error.value = normalizeAppError(e).message;
  } finally {
    saving.value = false;
  }
}

function diffStatsText(diff: FileDiffPayload | null): string {
  if (!diff) return "";
  return `+${diff.stats.additions} -${diff.stats.deletions}`;
}
</script>

<template>
  <div class="merge-text-view">
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
