<script setup lang="ts">
import { computed, ref, watch, onMounted, onUnmounted } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import type { UnmergedFileEntry, MergeFileInfo, MergeSessionPayload } from "../../types";
import { gitMergeFile } from "../../services/git";
import {
  mergeSemanticApply,
  mergeSemanticSession,
  mergeSemanticTarget,
  mergeSemanticValidate,
  listenMergeProgress,
} from "../../services/merge";
import type { MergeProgressEvent } from "../../services/merge";
import { normalizeAppError } from "../../services/errors";
import { useMergeResolution } from "../../composables/useMergeResolution";
import { t } from "../../i18n";
import {
  conflictCodeLabel,
  sharedBaseLabel,
} from "./mergeUi";
import MergeTextView from "./MergeTextView.vue";
import MergeSemanticView from "./MergeSemanticView.vue";

const props = defineProps<{
  file: UnmergedFileEntry;
}>();

const emit = defineEmits<{
  (e: "resolved"): void;
  (e: "back"): void;
}>();

const mergeInfo = ref<MergeFileInfo | null>(null);
const semanticSession = ref<MergeSessionPayload | null>(null);
const loading = ref(true);
const loadError = ref<string | null>(null);
const actionError = ref<string | null>(null);
const saving = ref(false);
const validatingApply = ref(false);
const applyValidationError = ref<string | null>(null);
const activeTab = ref<"semantic" | "text">("text");
const showConflictsOnly = ref(false);
const resolution = useMergeResolution();

const mergeProgress = ref<{ phase: string; current: number; total: number; elapsedMs: number }>({
  phase: "",
  current: 0,
  total: 4,
  elapsedMs: 0,
});

const mergePhaseLabel = computed(() => {
  switch (mergeProgress.value.phase) {
    case "fetchContent": return t("merge.phase.fetchContent");
    case "parseYaml": return t("merge.phase.parseYaml");
    case "buildSemantic": return t("merge.phase.buildStructured");
    case "done": return t("merge.phase.done");
    default: return t("merge.text.loading");
  }
});

const mergeProgressFraction = computed(() => {
  if (mergeProgress.value.total <= 0) return 0;
  return Math.min(1, (mergeProgress.value.current + 1) / mergeProgress.value.total);
});

let unlistenProgress: (() => void) | null = null;
let loadGeneration = 0;
let isUnmounted = false;
let validationGeneration = 0;

listenMergeProgress((evt: MergeProgressEvent) => {
  if (evt.requestKey !== `merge:${props.file.path}`) return;
  mergeProgress.value = {
    phase: evt.phase,
    current: evt.current,
    total: evt.total,
    elapsedMs: evt.elapsedMs,
  };
}).then((unlisten) => {
  unlistenProgress = unlisten;
});

onUnmounted(() => {
  isUnmounted = true;
  unlistenProgress?.();
});

const isBinaryOrSpecial = computed(() => {
  if (!mergeInfo.value) return false;
  return mergeInfo.value.isBinary || mergeInfo.value.isSubmodule;
});

const leftLabel = computed(() => mergeInfo.value?.leftLabel ?? "Ours");
const rightLabel = computed(() => mergeInfo.value?.rightLabel ?? "Theirs");
const displayBaseLabel = computed(() => sharedBaseLabel());
const displayConflictLabel = computed(() => conflictCodeLabel(props.file.conflictCode, props.file.semanticLabel));
const hasPendingStructuredChoices = computed(() =>
  resolution.fieldResolutions.value.size > 0
  || resolution.panelResolutions.value.size > 0
  || resolution.targetResolutions.value.size > 0,
);

const hasSemanticTab = computed(() => semanticSession.value?.semanticAvailable === true);

const conflictIdentity = computed(
  () => `${props.file.path}:${props.file.conflictCode}:${props.file.baseOid}:${props.file.leftOid}:${props.file.rightOid}`,
);

const conflictOids = computed(
  () => `${props.file.baseOid}-${props.file.leftOid}-${props.file.rightOid}`,
);

async function loadData() {
  const generation = ++loadGeneration;
  loading.value = true;
  loadError.value = null;
  actionError.value = null;
  applyValidationError.value = null;
  validatingApply.value = false;
  validationGeneration += 1;
  mergeInfo.value = null;
  semanticSession.value = null;
  activeTab.value = "text";
  resolution.reset();
  mergeProgress.value = { phase: "", current: 0, total: 4, elapsedMs: 0 };

  try {
    const [infoResult, semanticResult] = await Promise.allSettled([
      gitMergeFile(
        props.file.path,
        props.file.conflictCode,
        props.file.baseOid,
        props.file.leftOid,
        props.file.rightOid,
        props.file.lfs,
      ),
      mergeSemanticSession({
        filePath: props.file.path,
        baseOid: props.file.baseOid,
        leftOid: props.file.leftOid,
        rightOid: props.file.rightOid,
      }),
    ]);

    if (isUnmounted || generation !== loadGeneration) return;

    if (infoResult.status === "rejected") {
      throw infoResult.reason;
    }

    mergeInfo.value = infoResult.value;

    if (semanticResult.status === "fulfilled") {
      semanticSession.value = semanticResult.value;
      resolution.initializeSession(
        semanticResult.value.semanticAvailable ? semanticResult.value : null,
      );

      if (semanticResult.value.semanticAvailable) {
        activeTab.value = "semantic";
      }
    } else {
      semanticSession.value = {
        key: `merge-unavailable:${conflictIdentity.value}`,
        filePath: props.file.path,
        semanticAvailable: false,
        fallbackReason: normalizeAppError(semanticResult.reason).message,
      } as MergeSessionPayload;
      resolution.initializeSession(null);
    }
  } catch (e) {
    if (isUnmounted || generation !== loadGeneration) return;
    loadError.value = normalizeAppError(e).message;
  } finally {
    if (isUnmounted || generation !== loadGeneration) return;
    loading.value = false;
  }
}

watch(conflictIdentity, () => {
  void loadData();
}, { immediate: true });

async function materializePendingTargetResolutions() {
  if (!semanticSession.value?.semanticAvailable) return;
  const pendingTargetIds = resolution.pendingMaterializationTargetIds();
  for (const targetId of pendingTargetIds) {
    const side = resolution.targetResolutions.value.get(targetId);
    if (!side) continue;
    const inspector = await mergeSemanticTarget({
      mergeKey: semanticSession.value.key,
      targetId,
    });
    resolution.registerConflictFields(inspector);
    resolution.acceptTarget(targetId, side, inspector);
  }
}

function serializeResolutionMap(map: ReadonlyMap<string, string>): string {
  return Array.from(map.entries())
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([key, value]) => `${key}:${value}`)
    .join("|");
}

const resolutionSignature = computed(() => [
  semanticSession.value?.key ?? "",
  String(resolution.unresolvedCount.value),
  serializeResolutionMap(resolution.fieldResolutions.value),
  serializeResolutionMap(resolution.panelResolutions.value),
  serializeResolutionMap(resolution.targetResolutions.value),
].join("::"));

const canApplyStructured = computed(() =>
  resolution.canApply.value
  && !saving.value
  && !validatingApply.value
  && !applyValidationError.value,
);

async function validateSemanticResolution() {
  if (!semanticSession.value?.semanticAvailable) return;
  const generation = ++validationGeneration;
  validatingApply.value = true;
  applyValidationError.value = null;
  actionError.value = null;

  try {
    await materializePendingTargetResolutions();
    if (isUnmounted || generation !== validationGeneration) return;
    if (!resolution.canApply.value) {
      applyValidationError.value = t("merge.footer.applyBlocked");
      return;
    }
    await mergeSemanticValidate({
      mergeKey: semanticSession.value.key,
      filePath: props.file.path,
      resolutions: resolution.buildResolutions(),
    });
    if (isUnmounted || generation !== validationGeneration) return;
    applyValidationError.value = null;
  } catch (e) {
    if (isUnmounted || generation !== validationGeneration) return;
    applyValidationError.value = normalizeAppError(e).message;
  } finally {
    if (isUnmounted || generation !== validationGeneration) return;
    validatingApply.value = false;
  }
}

watch(resolutionSignature, () => {
  if (!semanticSession.value?.semanticAvailable || resolution.unresolvedCount.value > 0) {
    validationGeneration += 1;
    validatingApply.value = false;
    applyValidationError.value = null;
    return;
  }
  void validateSemanticResolution();
});

async function applySemanticResolution() {
  if (!semanticSession.value?.semanticAvailable) return;
  saving.value = true;
  actionError.value = null;
  try {
    await materializePendingTargetResolutions();
    if (!resolution.canApply.value) {
      throw new Error(t("merge.footer.applyBlocked"));
    }
    await mergeSemanticValidate({
      mergeKey: semanticSession.value.key,
      filePath: props.file.path,
      resolutions: resolution.buildResolutions(),
    });
    applyValidationError.value = null;
    await mergeSemanticApply({
      mergeKey: semanticSession.value.key,
      filePath: props.file.path,
      resolutions: resolution.buildResolutions(),
    });
    emit("resolved");
  } catch (e) {
    const message = normalizeAppError(e).message;
    applyValidationError.value = message;
    actionError.value = message;
  } finally {
    saving.value = false;
  }
}

async function confirmDiscardChanges(): Promise<boolean> {
  if (!hasPendingStructuredChoices.value) return true;
  return confirm(t("merge.leave.message"), {
    title: t("merge.leave.title"),
    kind: "warning",
  });
}

async function requestBack() {
  if (!(await confirmDiscardChanges())) return;
  emit("back");
}

function fileName(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1];
}

// ── Conflict navigation ────────────────────────────────────────
const semanticViewRef = ref<InstanceType<typeof MergeSemanticView> | null>(null);

function navigateConflict(direction: "prev" | "next") {
  semanticViewRef.value?.navigateConflict(direction);
}

function navigateTarget(direction: "prev" | "next") {
  semanticViewRef.value?.navigateTarget(direction);
}

function onKeydown(e: KeyboardEvent) {
  if (activeTab.value !== "semantic") return;
  if (e.key === "ArrowUp") {
    e.preventDefault();
    if (resolution.unresolvedCount.value > 0) navigateConflict("prev");
    else navigateTarget("prev");
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    if (resolution.unresolvedCount.value > 0) navigateConflict("next");
    else navigateTarget("next");
  }
}

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKeydown);
});

defineExpose({
  confirmDiscardChanges,
});
</script>

<template>
  <div class="merge-resolution-panel">
    <div class="merge-resolution-header">
      <div class="merge-header-main">
        <button class="merge-back-btn" @click="requestBack" :title="t('merge.back.title')">
          <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
            <path d="M7.78 12.53a.75.75 0 0 1-1.06 0L2.47 8.28a.75.75 0 0 1 0-1.06l4.25-4.25a.75.75 0 0 1 1.06 1.06L4.81 7h7.44a.75.75 0 0 1 0 1.5H4.81l2.97 2.97a.75.75 0 0 1 0 1.06z"/>
          </svg>
        </button>
        <span class="merge-resolution-title">
          <span class="file-name">{{ fileName(file.path) }}</span>
          <span class="conflict-code-badge" :class="file.conflictCode === 'UU' ? 'conflict-both' : 'conflict-mixed'">
            {{ displayConflictLabel }}
          </span>
        </span>
      </div>

      <button
        v-if="hasSemanticTab && !isBinaryOrSpecial && !loading"
        class="merge-filter-conflicts-btn"
        :class="{ active: showConflictsOnly }"
        @click="showConflictsOnly = !showConflictsOnly"
        :title="t('merge.filter.conflictsOnly')"
      >
        <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
          <path d="M.75 3h14.5a.75.75 0 0 1 0 1.5H.75a.75.75 0 0 1 0-1.5ZM3 7.75A.75.75 0 0 1 3.75 7h8.5a.75.75 0 0 1 0 1.5h-8.5A.75.75 0 0 1 3 7.75Zm3 4a.75.75 0 0 1 .75-.75h2.5a.75.75 0 0 1 0 1.5h-2.5a.75.75 0 0 1-.75-.75Z"/>
        </svg>
        {{ t("merge.filter.conflictsOnly") }}
      </button>

      <div v-if="hasSemanticTab && !isBinaryOrSpecial && !loading" class="merge-tab-bar merge-header-tabs">
        <button class="merge-tab-btn" :class="{ active: activeTab === 'semantic' }" @click="activeTab = 'semantic'">{{ t("merge.tabs.structured") }}</button>
        <button class="merge-tab-btn" :class="{ active: activeTab === 'text' }" @click="activeTab = 'text'">{{ t("merge.tabs.text") }}</button>
      </div>
    </div>

    <div v-if="loading" class="merge-loading">
      <span class="merge-loading-text">{{ mergePhaseLabel }}</span>
      <div class="merge-progress-bar">
        <div class="merge-progress-fill" :style="{ width: mergeProgressFraction * 100 + '%' }" />
      </div>
      <span v-if="mergeProgress.elapsedMs > 0" class="merge-loading-elapsed">{{ mergeProgress.elapsedMs }}ms</span>
    </div>
    <div v-else-if="loadError" class="merge-action-error">{{ loadError }}</div>

    <template v-if="!loading && mergeInfo">
      <div v-if="isBinaryOrSpecial" class="merge-resolution-content" />

      <template v-else>
        <MergeSemanticView
          ref="semanticViewRef"
          v-if="activeTab === 'semantic' && hasSemanticTab && semanticSession"
          :session="semanticSession"
          :resolution="resolution"
          :left-label="leftLabel"
          :right-label="rightLabel"
          :show-conflicts-only="showConflictsOnly"
          class="merge-semantic-fill"
        />

        <div v-if="activeTab === 'text' || !hasSemanticTab" class="merge-resolution-content">
          <MergeTextView
            :merge-info="mergeInfo"
            :file-path="file.path"
            :conflict-oids="conflictOids"
            :left-label="leftLabel"
            :right-label="rightLabel"
            :base-label="displayBaseLabel"
            @resolved="emit('resolved')"
          />
        </div>
      </template>

      <div v-if="activeTab === 'semantic' && hasSemanticTab" class="merge-semantic-footer-bar">
        <span v-if="resolution.unresolvedCount.value > 0" class="merge-unresolved-count">
          {{ t("merge.footer.unresolvedFields", resolution.unresolvedCount.value) }}
        </span>
        <span v-else class="merge-all-resolved">{{ t("merge.footer.allFieldsResolved") }}</span>

        <div v-if="resolution.unresolvedCount.value > 0" class="merge-conflict-nav">
          <button
            class="merge-action-btn merge-nav-btn"
            @click="navigateConflict('prev')"
            :title="t('merge.footer.prevConflict')"
          >
            {{ t("merge.footer.prevConflict") }}
            <kbd class="merge-shortcut-key">↑</kbd>
          </button>
          <button
            class="merge-action-btn merge-nav-btn"
            @click="navigateConflict('next')"
            :title="t('merge.footer.nextConflict')"
          >
            {{ t("merge.footer.nextConflict") }}
            <kbd class="merge-shortcut-key">↓</kbd>
          </button>
        </div>

        <button
          v-else
          class="merge-action-btn merge-continue-btn primary"
          :disabled="!canApplyStructured"
          :title="applyValidationError || t('merge.footer.applyHint')"
          @click="applySemanticResolution"
        >
          {{ saving ? t("merge.actions.applyingStructured") : t("merge.actions.applyStructured") }}
        </button>
      </div>
      <div v-if="activeTab === 'semantic' && hasSemanticTab && (applyValidationError || actionError)" class="merge-action-error merge-semantic-error">
        {{ actionError || applyValidationError }}
      </div>
    </template>
  </div>
</template>
