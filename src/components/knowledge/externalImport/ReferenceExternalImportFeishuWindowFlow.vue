<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { useCopyFeedback } from "../../../composables/useCopyFeedback";
import BaseButton from "../../ui/BaseButton.vue";
import BaseDropdown from "../../ui/BaseDropdown.vue";
import BaseSegmented from "../../ui/BaseSegmented.vue";
import FileTreeList from "../../explorer/FileTreeList.vue";
import type {
  ReferenceExternalImportFeishuTreeRowModel,
  ReferenceExternalImportFeishuWindowModel,
} from "./referenceExternalImportModels";

const props = defineProps<{
  model: ReferenceExternalImportFeishuWindowModel;
}>();

const emit = defineEmits<{
  (e: "update:auth-mode", value: string): void;
  (e: "update:app-id", value: string): void;
  (e: "update:app-secret", value: string): void;
  (e: "update:open-base-url", value: string): void;
  (e: "update:persistence-mode", value: string): void;
  (e: "test"): void;
  (e: "authorize"): void;
  (e: "update:space-id", value: string): void;
  (e: "use-space-root"): void;
  (e: "toggle-node", key: string): void;
  (e: "toggle-selection", key: string): void;
  (e: "delete"): void;
  (e: "cancel-authorization"): void;
  (e: "cancel-import"): void;
  (e: "start-import"): void;
}>();

const currentStep = ref(0);
const copiedCallbackUrl = ref("");
const { copied: callbackCopied, copyText: copyCallbackText } =
  useCopyFeedback();

watch(
  () => props.model.isRunning,
  (running) => {
    if (running) {
      currentStep.value = 2;
    }
  },
  { immediate: true },
);

const canContinue = computed(() => {
  if (currentStep.value === 0) return props.model.canContinueConnection;
  if (currentStep.value === 1) return !!props.model.spaceId.trim();
  return false;
});

const canGoBack = computed(() =>
  currentStep.value > 0 && !props.model.isRunning && !props.model.waitingForAuthorization,
);

function openStep(index: number) {
  if (props.model.isRunning || props.model.waitingForAuthorization) return;
  if (index < 0 || index > 2) return;
  if (index > currentStep.value) return;
  currentStep.value = index;
}

function goNext() {
  if (!canContinue.value || currentStep.value >= 2) return;
  currentStep.value += 1;
}

function goBack() {
  if (!canGoBack.value) return;
  currentStep.value -= 1;
}

const treeEmptyText = computed(() => {
  if (!props.model.spaceId.trim()) return props.model.treeEmptyText;
  return props.model.nodeLoading ? t("common.loading") : props.model.treeEmptyText;
});

async function copyCallbackUrl(value: string) {
  const normalized = value.trim();
  if (!normalized) return;
  const copied = await copyCallbackText(normalized);
  if (copied) {
    copiedCallbackUrl.value = normalized;
  }
}

function treeIndentPx(depth: number) {
  if (depth <= 0) return 10;
  return 10 + depth * 14;
}

function asTreeRow(item: { key: string }): ReferenceExternalImportFeishuTreeRowModel {
  return item as ReferenceExternalImportFeishuTreeRowModel;
}
</script>

<template>
  <div class="reference-feishu-flow">
    <div class="reference-feishu-flow-steps" role="tablist" :aria-label="t('knowledge.feishuReference.title')">
      <button
        v-for="(step, index) in model.steps"
        :key="step.key"
        type="button"
        class="reference-feishu-flow-step"
        :class="{ active: index === currentStep, done: index < currentStep }"
        :disabled="index > currentStep || model.isRunning || model.waitingForAuthorization"
        @click="openStep(index)"
      >
        <span class="reference-feishu-flow-step-index">{{ index + 1 }}</span>
        <span class="reference-feishu-flow-step-label">{{ step.label }}</span>
      </button>
    </div>

    <section v-if="currentStep === 0" class="reference-feishu-flow-section">
      <div class="reference-feishu-flow-heading">
        <div class="reference-feishu-flow-title">{{ model.steps[0].label }}</div>
      </div>

      <div class="reference-feishu-flow-toolbar">
        <BaseSegmented
          :model-value="model.authMode"
          size="sm"
          :options="model.authModeOptions"
          :aria-label="t('knowledge.feishuReference.window.authMode')"
          @update:model-value="emit('update:auth-mode', $event)"
        />
      </div>

      <div class="reference-feishu-flow-grid">
        <label class="reference-feishu-flow-field">
          <span class="reference-feishu-flow-label">{{ t("knowledge.feishuReference.window.appId") }}</span>
          <input
            :value="model.appId"
            class="reference-feishu-flow-input"
            :disabled="model.authDisabled"
            :placeholder="model.appIdPlaceholder"
            @input="emit('update:app-id', ($event.target as HTMLInputElement).value)"
          />
        </label>

        <label class="reference-feishu-flow-field">
          <span class="reference-feishu-flow-label">{{ t("knowledge.feishuReference.window.appSecret") }}</span>
          <input
            :value="model.appSecret"
            class="reference-feishu-flow-input"
            type="password"
            :disabled="model.authDisabled"
            :placeholder="model.appSecretPlaceholder"
            @input="emit('update:app-secret', ($event.target as HTMLInputElement).value)"
          />
        </label>

        <label class="reference-feishu-flow-field wide">
          <span class="reference-feishu-flow-label">{{ t("knowledge.feishuReference.window.openBaseUrl") }}</span>
          <input
            :value="model.openBaseUrl"
            class="reference-feishu-flow-input"
            :disabled="model.authDisabled"
            @input="emit('update:open-base-url', ($event.target as HTMLInputElement).value)"
          />
        </label>
      </div>

      <div v-if="model.showOauthSettings" class="reference-feishu-flow-stack">
        <div class="reference-feishu-flow-label">{{ t("knowledge.feishuReference.window.persistenceMode") }}</div>
        <BaseSegmented
          :model-value="model.persistenceMode"
          size="sm"
          :options="model.persistenceModeOptions"
          @update:model-value="emit('update:persistence-mode', $event)"
        />
        <div class="reference-feishu-flow-hint">{{ model.persistenceHint }}</div>
      </div>

      <div v-if="model.callbackUrls.length" class="reference-feishu-flow-stack">
        <div class="reference-feishu-flow-hint">{{ model.oauthAdminHint }}</div>
        <div class="reference-feishu-flow-hint">{{ model.oauthRedirectHint }}</div>
        <button
          v-for="callbackUrl in model.callbackUrls"
          :key="callbackUrl"
          type="button"
          class="reference-feishu-flow-callback"
          :class="{ copied: callbackCopied && copiedCallbackUrl === callbackUrl }"
          :title="
            callbackCopied && copiedCallbackUrl === callbackUrl
              ? t('common.copied')
              : t('common.clickToCopy')
          "
          @click="void copyCallbackUrl(callbackUrl)"
        >
          <span class="reference-feishu-flow-callback-value">{{ callbackUrl }}</span>
          <span class="reference-feishu-flow-copy-indicator">
            {{
              callbackCopied && copiedCallbackUrl === callbackUrl
                ? t("common.copied")
                : t("common.clickToCopy")
            }}
          </span>
        </button>
      </div>

      <div v-if="model.missingScopesHint" class="reference-feishu-flow-hint warning">
        {{ model.missingScopesHint }}
      </div>

      <div class="reference-feishu-flow-actions">
        <BaseButton
          v-if="model.canCancelAuthorization"
          :disabled="model.cancelAuthorizationDisabled"
          @click="emit('cancel-authorization')"
        >
          {{ model.cancelAuthorizationLabel }}
        </BaseButton>
        <BaseButton
          v-else-if="model.showAuthorize"
          :disabled="!model.canAuthorize"
          @click="emit('authorize')"
        >
          {{ model.authorizeLabel }}
        </BaseButton>
        <BaseButton
          v-if="model.showTest"
          :disabled="!model.canTest"
          @click="emit('test')"
        >
          {{ model.testLabel }}
        </BaseButton>
        <BaseButton
          variant="primary"
          :disabled="!canContinue"
          @click="goNext"
        >
          {{ t("onboarding.next") }}
        </BaseButton>
      </div>
    </section>

    <section v-else-if="currentStep === 1" class="reference-feishu-flow-section">
      <div class="reference-feishu-flow-heading">
        <div class="reference-feishu-flow-title">{{ model.steps[1].label }}</div>
        <div class="reference-feishu-flow-hint">{{ t("knowledge.feishuReference.window.scopeHint") }}</div>
      </div>

      <div class="reference-feishu-flow-scope">
        <div class="reference-feishu-flow-field">
          <span class="reference-feishu-flow-label">{{ t("knowledge.feishuReference.window.space") }}</span>
          <BaseDropdown
            class="reference-feishu-flow-dropdown"
            :model-value="model.spaceId"
            size="md"
            :disabled="!model.spaceOptions.length || model.isRunning"
            :options="model.spaceOptions"
            :placeholder="model.spacePlaceholder"
            :aria-label="t('knowledge.feishuReference.window.space')"
            @update:model-value="emit('update:space-id', $event)"
          />
        </div>

        <div class="reference-feishu-flow-scope-summary">
          <span class="reference-feishu-flow-label">{{ t("knowledge.feishuReference.window.selectedRoot") }}</span>
          <div class="reference-feishu-flow-value">{{ model.selectedScopeLabel }}</div>
          <div class="reference-feishu-flow-hint">{{ model.selectedScopeHint }}</div>
        </div>
      </div>

      <div class="reference-feishu-flow-toolbar">
        <BaseButton
          :disabled="!model.canUseSpaceRoot"
          @click="emit('use-space-root')"
        >
          {{ model.useSpaceRootLabel }}
        </BaseButton>
      </div>

      <div class="reference-feishu-flow-tree">
        <div v-if="model.nodeError" class="reference-feishu-flow-tree-empty error">{{ model.nodeError }}</div>
        <div v-else-if="!model.treeRows.length" class="reference-feishu-flow-tree-empty">
          {{ treeEmptyText }}
        </div>
        <FileTreeList
          v-else
          class="reference-feishu-flow-tree-list"
          :items="model.treeRows"
          :row-height="30"
        >
          <template #item="{ item }">
            <div
              v-for="row in [asTreeRow(item)]"
              :key="row.key"
              class="reference-feishu-flow-tree-row-shell"
              :class="{ selected: row.selected, disabled: row.disabled }"
            >
              <div
                class="reference-feishu-flow-tree-row"
                :style="{ paddingLeft: `${treeIndentPx(row.depth)}px` }"
              >
                <button
                  v-if="row.canExpand"
                  type="button"
                  class="reference-feishu-flow-tree-branch"
                  :aria-label="
                    row.expanded
                      ? t('merge.tree.toggleCollapse', row.title)
                      : t('merge.tree.toggleExpand', row.title)
                  "
                  :title="
                    row.expanded
                      ? t('merge.tree.toggleCollapse', row.title)
                      : t('merge.tree.toggleExpand', row.title)
                  "
                  :disabled="row.disabled"
                  @click.stop="emit('toggle-node', row.key)"
                >
                  <svg
                    class="reference-feishu-flow-tree-chevron"
                    :class="{ open: row.expanded }"
                    viewBox="0 0 16 16"
                    width="10"
                    height="10"
                    fill="currentColor"
                    aria-hidden="true"
                  >
                    <path d="M6.22 3.22a.75.75 0 0 1 1.06 0l4.25 4.25a.75.75 0 0 1 0 1.06l-4.25 4.25a.75.75 0 0 1-1.06-1.06L9.94 8 6.22 4.28a.75.75 0 0 1 0-1.06z" />
                  </svg>
                </button>
                <span
                  v-else
                  class="reference-feishu-flow-tree-branch-spacer"
                  aria-hidden="true"
                ></span>

                <span
                  class="reference-feishu-flow-tree-kind-icon"
                  :class="{ folder: row.canExpand, open: row.canExpand && row.expanded }"
                  aria-hidden="true"
                >
                  <svg
                    v-if="row.canExpand"
                    viewBox="0 0 16 16"
                    width="13"
                    height="13"
                    fill="none"
                  >
                    <path
                      v-if="!row.expanded"
                      d="M2.25 4.5A1.25 1.25 0 0 1 3.5 3.25h2.1c.32 0 .62.13.84.36l.8.82c.14.15.34.23.55.23h4.71A1.25 1.25 0 0 1 13.75 5.9v5.6a1.25 1.25 0 0 1-1.25 1.25H3.5a1.25 1.25 0 0 1-1.25-1.25V4.5Z"
                      fill="currentColor"
                    />
                    <template v-else>
                      <path
                        d="M2.25 5.2A1.25 1.25 0 0 1 3.5 3.95h2.04c.32 0 .62.13.84.36l.78.8c.14.15.34.23.55.23h4.79A1.25 1.25 0 0 1 13.75 6.6v.3"
                        stroke="currentColor"
                        stroke-width="1.2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      />
                      <path
                        d="M2.18 7.15A1.25 1.25 0 0 1 3.39 6.2h9.82a.75.75 0 0 1 .73.94l-.82 3.36a1.25 1.25 0 0 1-1.21.95H2.93a1.25 1.25 0 0 1-1.21-1.55l.46-1.85Z"
                        stroke="currentColor"
                        stroke-width="1.2"
                        stroke-linejoin="round"
                      />
                    </template>
                  </svg>
                  <svg
                    v-else
                    viewBox="0 0 16 16"
                    width="13"
                    height="13"
                    fill="none"
                  >
                    <path
                      d="M5 2.75h4.55c.3 0 .58.12.8.33l1.57 1.57c.21.22.33.5.33.8V12A1.25 1.25 0 0 1 11 13.25H5A1.25 1.25 0 0 1 3.75 12V4A1.25 1.25 0 0 1 5 2.75Z"
                      stroke="currentColor"
                      stroke-width="1.2"
                      stroke-linejoin="round"
                    />
                    <path
                      d="M9.5 2.9V5a.5.5 0 0 0 .5.5h2.1"
                      stroke="currentColor"
                      stroke-width="1.2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                </span>

                <button
                  type="button"
                  class="reference-feishu-flow-tree-main"
                  :class="{ selected: row.selected }"
                  :disabled="row.disabled"
                  :aria-pressed="row.selected"
                  :title="row.pathLabel"
                  @click="emit('toggle-selection', row.key)"
                >
                  <span class="reference-feishu-flow-tree-title">{{ row.title }}</span>
                </button>
              </div>
            </div>
          </template>
        </FileTreeList>
      </div>

      <div class="reference-feishu-flow-actions">
        <BaseButton @click="goBack">
          {{ t("common.back") }}
        </BaseButton>
        <BaseButton
          variant="primary"
          :disabled="!canContinue"
          @click="goNext"
        >
          {{ t("onboarding.next") }}
        </BaseButton>
      </div>
    </section>

    <section v-else class="reference-feishu-flow-section">
      <div class="reference-feishu-flow-heading">
        <div class="reference-feishu-flow-title">{{ model.steps[2].label }}</div>
        <div class="reference-feishu-flow-hint">{{ t("knowledge.feishuReference.window.importHint") }}</div>
      </div>

      <div class="reference-feishu-flow-progress-header">
        <span class="reference-feishu-flow-progress-title">{{ model.stageTitle }}</span>
        <span class="reference-feishu-flow-progress-value">{{ model.progressLabel }}</span>
      </div>
      <div class="reference-feishu-flow-progress-track" aria-hidden="true">
        <div
          class="reference-feishu-flow-progress-fill"
          :style="{ width: `${model.progressRatio * 100}%` }"
        />
      </div>
      <div class="reference-feishu-flow-detail">{{ model.detail }}</div>

      <div class="reference-feishu-flow-rows">
        <div
          v-for="row in model.rows"
          :key="row.label"
          class="reference-feishu-flow-row"
        >
          <span>{{ row.label }}</span>
          <span :class="{ mono: row.mono }">{{ row.value }}</span>
        </div>
      </div>

      <div v-if="model.currentItem" class="reference-feishu-flow-current">
        <div class="reference-feishu-flow-current-label">{{ model.currentItemLabel }}</div>
        <div class="reference-feishu-flow-current-value">{{ model.currentItem }}</div>
      </div>

      <div class="reference-feishu-flow-actions">
        <BaseButton
          v-if="canGoBack"
          @click="goBack"
        >
          {{ t("common.back") }}
        </BaseButton>
        <BaseButton
          v-if="model.canDelete"
          variant="danger"
          @click="emit('delete')"
        >
          {{ model.deleteLabel }}
        </BaseButton>
        <BaseButton
          v-if="model.canCancelAuthorization"
          :disabled="model.cancelAuthorizationDisabled"
          @click="emit('cancel-authorization')"
        >
          {{ model.cancelAuthorizationLabel }}
        </BaseButton>
        <BaseButton
          v-else-if="model.canCancelImport"
          :disabled="model.cancelImportDisabled"
          @click="emit('cancel-import')"
        >
          {{ model.cancelImportLabel }}
        </BaseButton>
        <BaseButton
          v-else
          variant="primary"
          :disabled="model.primaryDisabled"
          @click="emit('start-import')"
        >
          {{ model.primaryLabel }}
        </BaseButton>
      </div>
    </section>
  </div>
</template>

<style scoped>
.reference-feishu-flow {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.reference-feishu-flow-hint,
.reference-feishu-flow-detail {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.reference-feishu-flow-steps {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 10px;
}

.reference-feishu-flow-step {
  padding: 0 0 10px;
  border: none;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  background: transparent;
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--text-secondary);
  cursor: pointer;
  text-align: left;
}

.reference-feishu-flow-step.active,
.reference-feishu-flow-step.done {
  color: var(--text-color);
  border-bottom-color: color-mix(in srgb, var(--accent-color) 38%, var(--border-color));
}

.reference-feishu-flow-step:disabled {
  cursor: default;
}

.reference-feishu-flow-step-index {
  width: 20px;
  height: 20px;
  border-radius: 999px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  font-weight: 600;
  background: color-mix(in srgb, var(--panel-bg) 80%, var(--input-bg) 20%);
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
}

.reference-feishu-flow-step.active .reference-feishu-flow-step-index,
.reference-feishu-flow-step.done .reference-feishu-flow-step-index {
  background: color-mix(in srgb, var(--accent-soft) 72%, var(--panel-bg) 28%);
  border-color: color-mix(in srgb, var(--accent-color) 28%, var(--border-color));
  color: var(--accent-color);
}

.reference-feishu-flow-step-label,
.reference-feishu-flow-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.reference-feishu-flow-section {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.reference-feishu-flow-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.reference-feishu-flow-field {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.reference-feishu-flow-field.wide {
  grid-column: 1 / -1;
}

.reference-feishu-flow-label,
.reference-feishu-flow-current-label {
  font-size: 11px;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.reference-feishu-flow-input {
  width: 100%;
  min-height: 34px;
  padding: 0 12px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color) 18%);
  color: var(--text-color);
  font-size: 13px;
  box-sizing: border-box;
}

.reference-feishu-flow-input:focus {
  outline: none;
  border-color: var(--accent-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.reference-feishu-flow-toolbar,
.reference-feishu-flow-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.reference-feishu-flow-stack {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.reference-feishu-flow-callback {
  width: 100%;
  min-width: 0;
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 80%, var(--panel-bg) 20%);
  color: inherit;
  display: inline-flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  font-size: 12px;
  font-family: var(--font-mono-identifier);
  text-align: left;
  cursor: pointer;
  transition:
    border-color 0.15s ease,
    color 0.15s ease;
}

.reference-feishu-flow-callback:focus-visible {
  outline: none;
  border-color: color-mix(in srgb, var(--accent-color) 36%, var(--border-color));
}

.reference-feishu-flow-callback-value {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.reference-feishu-flow-callback:hover .reference-feishu-flow-callback-value,
.reference-feishu-flow-callback:focus-visible .reference-feishu-flow-callback-value {
  color: var(--text-color);
  text-decoration: underline;
  text-underline-offset: 0.16em;
}

.reference-feishu-flow-copy-indicator {
  flex-shrink: 0;
  font-size: 11px;
  font-family: var(--font-ui);
  color: var(--text-secondary);
  white-space: nowrap;
}

.reference-feishu-flow-callback.copied .reference-feishu-flow-copy-indicator {
  color: var(--accent-color);
}

.reference-feishu-flow-hint.warning {
  color: var(--status-danger-fg, var(--text-secondary));
}

.reference-feishu-flow-value {
  font-size: 13px;
  color: var(--text-color);
  font-weight: 600;
}

.reference-feishu-flow-scope {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.reference-feishu-flow-scope-summary {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.reference-feishu-flow-dropdown :deep(.base-dropdown-trigger) {
  justify-content: space-between;
  text-align: left;
}

.reference-feishu-flow-dropdown :deep(.base-dropdown-value) {
  text-align: left;
}

.reference-feishu-flow-dropdown :deep(.base-dropdown-menu) {
  left: 0;
  right: auto;
  min-width: 100%;
}

.reference-feishu-flow-tree {
  min-height: 320px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 82%, var(--panel-bg) 18%);
  overflow: auto;
}

.reference-feishu-flow-tree-empty {
  min-height: 260px;
  display: flex;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 16px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.reference-feishu-flow-tree-empty.error {
  color: var(--status-danger-fg);
}

.reference-feishu-flow-tree-list {
  padding: 4px 0;
}

.reference-feishu-flow-tree-row-shell {
  display: flex;
  align-items: stretch;
  min-width: 0;
  min-height: 30px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  background: transparent;
  transition: background 0.12s ease;
}

.reference-feishu-flow-tree-row-shell:hover {
  background: var(--hover-bg);
}

.reference-feishu-flow-tree-row-shell.selected,
.reference-feishu-flow-tree-row-shell.selected:hover {
  background: var(--active-bg);
}

.reference-feishu-flow-tree-row-shell.disabled {
  opacity: 0.72;
}

.reference-feishu-flow-tree-row {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  min-width: 0;
  min-height: 30px;
  padding: 2px 12px 2px 10px;
}

.reference-feishu-flow-tree-branch,
.reference-feishu-flow-tree-branch-spacer,
.reference-feishu-flow-tree-kind-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  min-width: 14px;
  height: 16px;
  flex-shrink: 0;
  align-self: center;
}

.reference-feishu-flow-tree-branch {
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
}

.reference-feishu-flow-tree-branch:hover:not(:disabled) {
  background: color-mix(in srgb, var(--hover-bg) 85%, transparent);
  color: var(--text-color);
}

.reference-feishu-flow-tree-branch:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -1px;
}

.reference-feishu-flow-tree-branch:disabled {
  opacity: 0.5;
  cursor: default;
}

.reference-feishu-flow-tree-chevron {
  opacity: 0.72;
  transition: transform 0.15s ease;
}

.reference-feishu-flow-tree-chevron.open {
  transform: rotate(90deg);
}

.reference-feishu-flow-tree-kind-icon {
  color: color-mix(in srgb, var(--text-secondary) 84%, transparent);
}

.reference-feishu-flow-tree-kind-icon.folder {
  color: color-mix(in srgb, var(--accent-color) 38%, var(--text-secondary) 62%);
}

.reference-feishu-flow-tree-kind-icon.folder.open {
  color: color-mix(in srgb, var(--accent-color) 54%, var(--text-secondary) 46%);
}

.reference-feishu-flow-tree-main {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 1;
  min-width: 0;
  min-height: 26px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-color);
  cursor: pointer;
  font: inherit;
  text-align: left;
}

.reference-feishu-flow-tree-main:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.reference-feishu-flow-tree-main.selected .reference-feishu-flow-tree-title {
  color: var(--text-color);
  font-weight: 600;
}

.reference-feishu-flow-tree-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
}

.reference-feishu-flow-progress-header {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px;
}

.reference-feishu-flow-progress-title {
  font-size: 18px;
  font-weight: 600;
  color: var(--text-color);
}

.reference-feishu-flow-progress-value {
  font-size: 20px;
  font-weight: 700;
  color: var(--text-color);
}

.reference-feishu-flow-progress-track {
  position: relative;
  height: 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--input-bg) 76%, var(--border-color) 24%);
  overflow: hidden;
}

.reference-feishu-flow-progress-fill {
  position: absolute;
  inset: 0 auto 0 0;
  min-width: 0;
  border-radius: inherit;
  background: linear-gradient(
    90deg,
    color-mix(in srgb, var(--accent-color) 74%, #ffffff 26%),
    var(--accent-color)
  );
}

.reference-feishu-flow-rows {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-feishu-flow-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.reference-feishu-flow-row span:last-child {
  color: var(--text-color);
  font-weight: 600;
  text-align: right;
}

.reference-feishu-flow-row span.mono,
.reference-feishu-flow-current-value {
  font-family: var(--font-mono-identifier);
  word-break: break-word;
}

.reference-feishu-flow-current {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-top: 12px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.reference-feishu-flow-current-value {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-color);
}

.reference-feishu-flow-actions {
  justify-content: flex-end;
  padding-top: 14px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

@media (max-width: 980px) {
  .reference-feishu-flow-steps,
  .reference-feishu-flow-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .reference-feishu-flow-step {
    padding-bottom: 8px;
  }
}
</style>
