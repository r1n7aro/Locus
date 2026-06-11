<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emit as emitTauriEvent, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { X } from "lucide";
import { t } from "../i18n";
import LucideIcon from "./icons/LucideIcon.vue";
import UnityCurveEditor from "./unity/editors/UnityCurveEditor.vue";
import UnityGradientEditor from "./unity/editors/UnityGradientEditor.vue";
import {
  getUnityValueEditorWindowPayload,
  UNITY_VALUE_EDITOR_COMMITTED_EVENT,
  UNITY_VALUE_EDITOR_PAYLOAD_EVENT,
  UNITY_VALUE_EDITOR_READY_EVENT,
  type UnityValueEditorPayload,
} from "../services/unityValueEditorWindow";
import {
  readUnitySerializedProperty,
  writeUnitySerializedProperty,
} from "../services/unitySerializedProperty";
import { isUnityConnectionError, normalizeAppError } from "../services/errors";
import {
  unityAnimationCurveValue,
  unityGradientValue,
} from "./unity/unitySerializedValue";

const PREVIEW_WRITE_INTERVAL_MS = 90;

const payload = ref<UnityValueEditorPayload | null>(null);
const loading = ref(false);
const loadError = ref("");
const editable = ref(false);
const originalValue = ref<unknown>(null);
const dirty = ref(false);
const saving = ref(false);
const saveError = ref("");
const livePreviewBroken = ref(false);

let latestEdit: Record<string, unknown> | null = null;
let previewTimer: number | null = null;
let previewSent = false;
let loadRun = 0;
let unlistenPayload: UnlistenFn | null = null;

const curveValue = computed(() =>
  payload.value?.kind === "curve" ? unityAnimationCurveValue(originalValue.value) : null,
);
const gradientValue = computed(() =>
  payload.value?.kind === "gradient" ? unityGradientValue(originalValue.value) : null,
);
const windowTitle = computed(() =>
  payload.value?.kind === "gradient"
    ? t("unity.valueEditor.title.gradient")
    : t("unity.valueEditor.title.curve"),
);
const targetLabel = computed(() => {
  const target = payload.value?.target;
  if (!target) return "";
  const object = target.objectPath || target.path || target.scenePath || target.guid || target.kind;
  return `${object ?? ""} · ${payload.value?.label || target.propertyPath || ""}`;
});

function clearPreviewTimer() {
  if (previewTimer === null) return;
  window.clearTimeout(previewTimer);
  previewTimer = null;
}

async function applyWindowPayload(next: UnityValueEditorPayload) {
  if (dirty.value && payload.value) {
    const discard = window.confirm(t("unity.valueEditor.discardConfirm"));
    if (!discard) return;
    await restoreOriginalPreview();
  }
  clearPreviewTimer();
  latestEdit = null;
  previewSent = false;
  dirty.value = false;
  saveError.value = "";
  livePreviewBroken.value = false;
  payload.value = next;
  await loadCurrentValue();
}

async function loadCurrentValue() {
  const target = payload.value?.target;
  if (!target) return;
  const run = ++loadRun;
  loading.value = true;
  loadError.value = "";
  try {
    const result = await readUnitySerializedProperty({ target, maxDepth: 1, maxArrayItems: 0 });
    if (run !== loadRun) return;
    if (!result.ok) throw new Error(result.message || t("unity.valueEditor.loadFailed"));
    originalValue.value = result.value;
    editable.value = result.editable !== false;
  } catch (error) {
    if (run !== loadRun) return;
    const normalized = normalizeAppError(error);
    loadError.value = isUnityConnectionError(normalized)
      ? t("asset.preview.unityConnectionRequired")
      : normalized.message;
    editable.value = false;
  } finally {
    if (run === loadRun) loading.value = false;
  }
}

function handleEditorChange(next: Record<string, unknown>) {
  if (!editable.value) return;
  latestEdit = next;
  dirty.value = true;
  saveError.value = "";
  if (livePreviewBroken.value || previewTimer !== null) return;
  previewTimer = window.setTimeout(() => {
    previewTimer = null;
    void flushPreview();
  }, PREVIEW_WRITE_INTERVAL_MS);
}

async function flushPreview() {
  const target = payload.value?.target;
  const value = latestEdit;
  if (!target || value == null || livePreviewBroken.value) return;
  try {
    await writeUnitySerializedProperty({ target, value, writeMode: "preview" });
    previewSent = true;
  } catch (error) {
    // Preview is best-effort; keep editing locally and commit at the end.
    livePreviewBroken.value = true;
    console.warn("[UnityValueEditorWindow] live preview disabled:", error);
  }
}

async function restoreOriginalPreview() {
  const target = payload.value?.target;
  if (!target || !previewSent || originalValue.value == null) return;
  try {
    await writeUnitySerializedProperty({
      target,
      value: originalValue.value,
      writeMode: "preview",
    });
  } catch (error) {
    console.warn("[UnityValueEditorWindow] preview restore failed:", error);
  }
  previewSent = false;
}

async function applyChanges() {
  const target = payload.value?.target;
  const kind = payload.value?.kind;
  if (!target || !kind || !dirty.value || latestEdit == null || saving.value) return;
  clearPreviewTimer();
  saving.value = true;
  saveError.value = "";
  try {
    const result = await writeUnitySerializedProperty({
      target,
      value: latestEdit,
      writeMode: "commit",
    });
    if (!result.ok) throw new Error(result.message || t("unity.valueEditor.saveFailed"));
    dirty.value = false;
    previewSent = false;
    void emitTauriEvent(UNITY_VALUE_EDITOR_COMMITTED_EVENT, {
      kind,
      target,
      propertyPath: target.propertyPath ?? "",
      value: latestEdit,
    }).catch(() => {});
    await closeWindow();
  } catch (error) {
    const normalized = normalizeAppError(error);
    saveError.value = isUnityConnectionError(normalized)
      ? t("asset.preview.unityConnectionRequired")
      : normalized.message;
  } finally {
    saving.value = false;
  }
}

async function cancelChanges() {
  clearPreviewTimer();
  await restoreOriginalPreview();
  await closeWindow();
}

async function closeWindow() {
  const currentWindow = getCurrentWindow();
  try {
    await currentWindow.close();
  } catch (error) {
    console.warn("[UnityValueEditorWindow] close failed:", error);
  }
  await currentWindow.destroy().catch(() => {});
}

function handleWindowKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape" || event.defaultPrevented) return;
  const target = event.target as HTMLElement | null;
  if (target?.closest("input, textarea, select, [contenteditable='true']")) return;
  void cancelChanges();
}

onMounted(async () => {
  const initial = getUnityValueEditorWindowPayload();
  if (initial) await applyWindowPayload(initial);
  window.addEventListener("keydown", handleWindowKeydown);
  unlistenPayload = await listen<UnityValueEditorPayload>(
    UNITY_VALUE_EDITOR_PAYLOAD_EVENT,
    (event) => {
      void applyWindowPayload(event.payload);
    },
  );
  void emitTauriEvent(UNITY_VALUE_EDITOR_READY_EVENT).catch(() => {});
});

onUnmounted(() => {
  window.removeEventListener("keydown", handleWindowKeydown);
  clearPreviewTimer();
  unlistenPayload?.();
  unlistenPayload = null;
});
</script>

<template>
  <div class="unity-value-editor-window">
    <div class="unity-value-editor-titlebar">
      <div class="unity-value-editor-title">
        <span class="unity-value-editor-title-main">
          {{ windowTitle }}
          <span v-if="dirty" class="unity-value-editor-dirty" :title="t('unity.valueEditor.dirty')">●</span>
        </span>
        <span class="unity-value-editor-title-path" :title="targetLabel">{{ targetLabel }}</span>
      </div>
      <span
        v-if="livePreviewBroken"
        class="unity-value-editor-preview-state"
        :title="t('unity.valueEditor.previewBroken')"
      >
        {{ t("unity.valueEditor.previewBrokenShort") }}
      </span>
      <button
        type="button"
        class="unity-value-editor-close"
        data-window-no-drag
        :title="t('app.win.close')"
        @pointerdown.stop
        @click="cancelChanges"
      >
        <LucideIcon :icon="X" :size="14" />
      </button>
    </div>

    <div class="unity-value-editor-body">
      <div v-if="loading" class="unity-value-editor-state">{{ t("unity.valueEditor.loading") }}</div>
      <div v-else-if="loadError" class="unity-value-editor-state error">{{ loadError }}</div>
      <template v-else-if="payload">
        <div v-if="!editable" class="unity-value-editor-readonly-banner">
          {{ t("unity.valueEditor.readonly") }}
        </div>
        <UnityCurveEditor
          v-if="payload.kind === 'curve'"
          :value="curveValue"
          :readonly="!editable"
          @change="handleEditorChange"
        />
        <UnityGradientEditor
          v-else
          :value="gradientValue"
          :readonly="!editable"
          @change="handleEditorChange"
        />
      </template>
      <div v-else class="unity-value-editor-state">{{ t("unity.valueEditor.missingTarget") }}</div>
    </div>

    <div class="unity-value-editor-footer">
      <span v-if="saveError" class="unity-value-editor-save-error" :title="saveError">{{ saveError }}</span>
      <span v-else class="unity-value-editor-footer-spacer" />
      <button type="button" class="unity-value-editor-button" @click="cancelChanges">
        {{ t("unity.valueEditor.cancel") }}
      </button>
      <button
        type="button"
        class="unity-value-editor-button primary"
        :disabled="!dirty || !editable || saving"
        @click="applyChanges"
      >
        {{ saving ? t("unity.valueEditor.saving") : t("unity.valueEditor.apply") }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.unity-value-editor-window {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--border-strong);
  background: var(--panel-bg);
  color: var(--text-color);
}

.unity-value-editor-titlebar {
  -webkit-app-region: drag;
  min-height: 38px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 10px 0 14px;
  border-bottom: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.unity-value-editor-title {
  min-width: 0;
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  gap: 8px;
}

.unity-value-editor-title-main {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.unity-value-editor-dirty {
  color: var(--accent-color);
  font-size: 10px;
}

.unity-value-editor-title-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  direction: rtl;
  text-align: left;
}

.unity-value-editor-preview-state,
.unity-value-editor-close {
  -webkit-app-region: no-drag;
}

.unity-value-editor-preview-state {
  flex-shrink: 0;
  padding: 2px 7px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  color: var(--text-secondary);
  font-size: 11px;
}

.unity-value-editor-close {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.unity-value-editor-close:hover,
.unity-value-editor-close:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-color);
  color: var(--text-color);
  outline: none;
}

.unity-value-editor-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px;
  overflow: auto;
}

.unity-value-editor-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 12px;
}

.unity-value-editor-state.error {
  color: var(--status-danger-fg);
}

.unity-value-editor-readonly-banner {
  padding: 5px 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--hover-bg) 50%, transparent);
  color: var(--text-secondary);
  font-size: 11px;
}

.unity-value-editor-footer {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 9px 14px;
  border-top: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.unity-value-editor-save-error {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--status-danger-fg);
  font-size: 11px;
}

.unity-value-editor-footer-spacer {
  flex: 1;
}

.unity-value-editor-button {
  min-height: 28px;
  padding: 0 14px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

.unity-value-editor-button:hover:not(:disabled) {
  background: var(--hover-bg);
}

.unity-value-editor-button.primary {
  border-color: color-mix(in srgb, var(--accent-color) 60%, var(--border-color));
  background: color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.unity-value-editor-button.primary:hover:not(:disabled) {
  background: color-mix(in srgb, var(--accent-color) 28%, transparent);
}

.unity-value-editor-button:disabled {
  opacity: 0.55;
  cursor: default;
}
</style>
