<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Download, Package, Terminal } from "lucide";
import { t } from "../../i18n";
import type {
  KnowledgeDocumentPatch,
  KnowledgeDocumentSummary,
  KnowledgeInjectMode,
  SkillManifest,
  SkillSurface,
} from "../../types";
import { skillSurfaceAllowsAuto, skillSurfaceAllowsCommand } from "../../types";
import { useSkills } from "../../composables/useSkills";
import {
  findSkillCommandConflict,
  isValidSkillCommandTrigger,
  normalizeSkillCommandTrigger,
  SKILL_COMMAND_NOTICE_OPERATION,
} from "../../composables/skillCommands";
import { useNotificationStore } from "../../stores/notification";
import {
  hintForInjectMode,
  labelForInjectMode,
} from "./knowledgeMetaLabels";
import LucideIcon from "../icons/LucideIcon.vue";
import {
  unityAssetIconClassForPath,
  unityAssetIconNodeForPath,
} from "../icons/unityAssetIcons";
import BaseDropdown from "../ui/BaseDropdown.vue";
import BaseButton from "../ui/BaseButton.vue";

const props = defineProps<{
  packageDocument: KnowledgeDocumentSummary;
  documents: KnowledgeDocumentSummary[];
  saveLoading?: boolean;
}>();

const emit = defineEmits<{
  (e: "selectDocument", document: KnowledgeDocumentSummary): void;
  (e: "updateConfig", patch: KnowledgeDocumentPatch): void;
  (e: "exportPackage", packageId: string): void;
}>();

const { skillItems, loadSkills } = useSkills();
const notificationStore = useNotificationStore();
const skillCommandDraft = ref("");

function normalizeRelativePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
}

function packageIdForDocument(document: KnowledgeDocumentSummary): string {
  if (document.type !== "skill") return "";
  if (document.externalSource?.provider !== "package") return "";
  const normalizedPath = normalizeRelativePath(document.path);
  return (
    document.externalSource.sourceId?.trim() ||
    normalizedPath.split("/").filter(Boolean)[0] ||
    ""
  );
}

function surfaceLabel(surface: SkillSurface | null | undefined): string {
  switch (surface) {
    case "auto":
      return t("knowledge.skill.surfaceAuto");
    case "both":
      return t("knowledge.skill.surfaceBoth");
    case "command":
      return t("knowledge.skill.surfaceCommand");
    default:
      return t("knowledge.skill.surfaceCommand");
  }
}

const skillSurfaceOptions = computed(() => [
  {
    value: "disabled",
    label: t("knowledge.skill.surfaceDisabled"),
    hint: t("knowledge.skill.surfaceDisabledHint"),
  },
  {
    value: "command",
    label: t("knowledge.skill.surfaceCommand"),
    hint: t("knowledge.skill.surfaceCommandHint"),
  },
  {
    value: "auto",
    label: t("knowledge.skill.surfaceAuto"),
    hint: t("knowledge.skill.surfaceAutoHint"),
  },
  {
    value: "both",
    label: t("knowledge.skill.surfaceBoth"),
    hint: t("knowledge.skill.surfaceBothHint"),
  },
]);

const injectModeOptions = computed(() => [
  {
    value: "none",
    label: labelForInjectMode("none"),
    hint: hintForInjectMode("none"),
  },
  {
    value: "path",
    label: labelForInjectMode("path"),
    hint: hintForInjectMode("path"),
  },
  {
    value: "excerpt",
    label: labelForInjectMode("excerpt"),
    hint: manifest.value?.hasL1 === false
      ? t("knowledge.skill.l1FallbackDescription")
      : hintForInjectMode("excerpt"),
  },
]);

function documentFileName(document: KnowledgeDocumentSummary): string {
  const normalizedPath = normalizeRelativePath(document.path);
  return normalizedPath.split("/").pop() || document.title || normalizedPath;
}

function documentIconNode(document: KnowledgeDocumentSummary) {
  return unityAssetIconNodeForPath(document.path || document.title, {
    isFolder: false,
  });
}

function documentIconClass(document: KnowledgeDocumentSummary) {
  return unityAssetIconClassForPath(document.path || document.title, {
    isFolder: false,
  });
}

function formatDateTime(value: number): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "-";
  return date.toLocaleString(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const packageId = computed(() => packageIdForDocument(props.packageDocument));

const manifest = computed<SkillManifest | null>(
  () =>
    skillItems.value.find(
      (item) =>
        item.kind === "package" &&
        (item.packageId === packageId.value || item.dirName === packageId.value),
    ) ?? null,
);

const packageDocuments = computed(() => {
  const id = packageId.value;
  if (!id) return [];
  const rootPath = `${id}/SKILL.md`;
  const prefix = `${id}/`;
  return props.documents
    .filter(
      (document) =>
        document.type === "skill" &&
        (normalizeRelativePath(document.path) === rootPath ||
          normalizeRelativePath(document.path).startsWith(prefix)),
    )
    .sort((left, right) => {
      const leftPath = normalizeRelativePath(left.path);
      const rightPath = normalizeRelativePath(right.path);
      if (leftPath === rootPath) return -1;
      if (rightPath === rootPath) return 1;
      return leftPath.localeCompare(rightPath, undefined, {
        sensitivity: "base",
        numeric: true,
      });
    });
});

const displayName = computed(
  () => manifest.value?.name?.trim() || packageId.value || props.packageDocument.title,
);
const description = computed(
  () =>
    props.packageDocument.summary?.trim() ||
    manifest.value?.skillDescription?.trim() ||
    manifest.value?.description?.trim() ||
    "",
);
const commandTrigger = computed(
  () =>
    props.packageDocument.commandTrigger?.trim() ||
    manifest.value?.commandTrigger?.trim() ||
    "",
);
const argumentHint = computed(
  () =>
    manifest.value?.argumentHint?.trim() ||
    props.packageDocument.argumentHint?.trim() ||
    "",
);
const packageVersion = computed(
  () => manifest.value?.packageVersion?.trim() || "-",
);
const packagePath = computed(() =>
  packageId.value ? `skill/${packageId.value}` : "skill",
);
const packageSourcePath = computed(
  () =>
    manifest.value?.relPath?.trim() ||
    props.packageDocument.externalSource?.locator?.trim() ||
    "-",
);
const enabledLabel = computed(() => {
  return packageEnabled.value
    ? t("knowledge.skillPackage.enabled")
    : t("knowledge.skillPackage.disabled");
});
const packageEnabled = computed(
  () => props.packageDocument.skillEnabled ?? manifest.value?.skillEnabled ?? true,
);
const packageSurface = computed<SkillSurface>(
  () => props.packageDocument.skillSurface ?? manifest.value?.skillSurface ?? "command",
);
const skillSurfaceValue = computed(() =>
  packageEnabled.value ? packageSurface.value : "disabled",
);
const surfaceText = computed(() => {
  if (!packageEnabled.value) return t("knowledge.skill.surfaceDisabled");
  return surfaceLabel(packageSurface.value);
});
const updatedLabel = computed(() =>
  formatDateTime(manifest.value?.updatedAt ?? props.packageDocument.updatedAt),
);
const injectMode = computed(
  () => props.packageDocument.injectMode ?? "none",
);
const injectModeDropdownLabel = computed(() =>
  labelForInjectMode(injectMode.value),
);
const fallbackSkillName = computed(
  () => packageId.value || displayName.value,
);
const currentSkillCommandTrigger = computed(() =>
  normalizeSkillCommandTrigger(commandTrigger.value, fallbackSkillName.value),
);
const showSkillCommandFields = computed(
  () =>
    packageEnabled.value &&
    skillSurfaceAllowsCommand(packageSurface.value),
);
const skillCommandInputDisabled = computed(
  () => !!props.saveLoading || !showSkillCommandFields.value,
);
const configControlsDisabled = computed(() => !!props.saveLoading);

const capabilityTags = computed(() => {
  const tags: string[] = [];
  if (
    packageEnabled.value &&
    skillSurfaceAllowsCommand(packageSurface.value) &&
    commandTrigger.value
  ) {
    tags.push(t("knowledge.skillPackage.command"));
  }
  if (packageEnabled.value && skillSurfaceAllowsAuto(packageSurface.value)) {
    tags.push(t("knowledge.skillPackage.auto"));
  }
  if (manifest.value?.hasUnity) tags.push(t("knowledge.skillPackage.unity"));
  if (manifest.value?.hasL0) tags.push("L0");
  if (manifest.value?.hasL1) tags.push("L1");
  if (manifest.value?.hasL2) tags.push("L2");
  return tags;
});

const infoRows = computed(() => [
  {
    label: t("knowledge.skillPackage.packageId"),
    value: packageId.value || "-",
  },
  {
    label: t("knowledge.skillPackage.version"),
    value: packageVersion.value,
  },
  {
    label: t("knowledge.skillPackage.argumentHint"),
    value: argumentHint.value || "-",
  },
  {
    label: t("knowledge.skillPackage.packagePath"),
    value: packagePath.value,
  },
  {
    label: t("knowledge.skillPackage.sourcePath"),
    value: packageSourcePath.value,
  },
  {
    label: t("knowledge.skillPackage.updatedAt"),
    value: updatedLabel.value,
  },
]);

watch(
  packageId,
  () => {
    void loadSkills();
  },
  { immediate: true },
);

watch(
  currentSkillCommandTrigger,
  (value) => {
    skillCommandDraft.value = value;
  },
  { immediate: true },
);

function showSkillCommandError(message: string) {
  notificationStore.addNotice("error", message, {
    operation: SKILL_COMMAND_NOTICE_OPERATION,
    replaceOperation: true,
    sticky: true,
  });
}

function onInjectModeChange(value: string) {
  if (!["none", "path", "excerpt"].includes(value)) return;
  emit("updateConfig", {
    injectMode: value as KnowledgeInjectMode,
    inheritInjectMode: false,
  });
}

function onSkillSurfaceChange(value: string) {
  notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
  if (value === "disabled") {
    emit("updateConfig", { skillEnabled: false });
    return;
  }

  const nextSurface = value as SkillSurface;
  emit("updateConfig", {
    skillEnabled: true,
    skillSurface: nextSurface,
    commandTrigger: skillSurfaceAllowsCommand(nextSurface)
      ? currentSkillCommandTrigger.value
      : commandTrigger.value || null,
  });
}

function persistSkillCommandTrigger() {
  if (skillCommandInputDisabled.value) return;
  const normalizedTrigger = normalizeSkillCommandTrigger(
    skillCommandDraft.value,
    fallbackSkillName.value,
  );
  if (!isValidSkillCommandTrigger(normalizedTrigger)) {
    showSkillCommandError(t("knowledge.skill.commandTriggerInvalid"));
    return;
  }

  const conflict = findSkillCommandConflict(normalizedTrigger, skillItems.value, {
    source: "app",
    dirName: manifest.value?.dirName ?? packageId.value,
  });
  if (conflict) {
    showSkillCommandError(
      conflict.type === "builtin"
        ? t("knowledge.skill.commandTriggerBuiltinConflict", conflict.command)
        : t(
            "knowledge.skill.commandTriggerSkillConflict",
            conflict.command,
            conflict.skillName ?? "",
          ),
    );
    return;
  }

  if (normalizedTrigger === currentSkillCommandTrigger.value) {
    notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
    skillCommandDraft.value = currentSkillCommandTrigger.value;
    return;
  }

  notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
  emit("updateConfig", { commandTrigger: normalizedTrigger });
}

function onSkillCommandKeydown(event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    persistSkillCommandTrigger();
    return;
  }

  if (event.key === "Escape") {
    skillCommandDraft.value = currentSkillCommandTrigger.value;
    notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
    (event.target as HTMLInputElement | null)?.blur();
  }
}

function onExportPackage() {
  if (!packageId.value) return;
  emit("exportPackage", packageId.value);
}
</script>

<template>
  <div class="skill-package-preview">
    <header class="skill-package-header">
      <div class="skill-package-title-row">
        <span class="skill-package-icon" aria-hidden="true">
          <LucideIcon :icon="Package" :size="18" :stroke-width="2" />
        </span>
        <div class="skill-package-title-main">
          <div class="skill-package-eyebrow">
            {{ t("knowledge.skillPackage.badge") }}
          </div>
          <h1 class="skill-package-title">{{ displayName }}</h1>
        </div>
      </div>
      <div class="skill-package-header-side">
        <BaseButton
          class="skill-package-header-action"
          type="button"
          :disabled="!packageId || saveLoading"
          :title="t('knowledge.skillPackage.export')"
          @click="onExportPackage"
        >
          <LucideIcon :icon="Download" :size="13" :stroke-width="2.2" />
          <span>{{ t("knowledge.skillPackage.export") }}</span>
        </BaseButton>
        <div class="skill-package-path">{{ packagePath }}</div>
      </div>
    </header>

    <main class="skill-package-body">
      <section class="skill-package-section">
        <div class="skill-package-section-title">
          {{ t("knowledge.skillPackage.description") }}
        </div>
        <p class="skill-package-description">
          {{
            description || t("knowledge.skillPackage.noDescription")
          }}
        </p>
      </section>

      <section class="skill-package-section">
        <div class="skill-package-section-title">
          {{ t("knowledge.skillPackage.config") }}
        </div>
        <div class="skill-package-config-grid">
          <div class="skill-package-config-row">
            <span class="skill-package-config-label">
              {{ t("knowledge.meta.injectMode") }}
            </span>
            <BaseDropdown
              class="skill-package-dropdown"
              :model-value="injectMode"
              :selected-label="injectModeDropdownLabel"
              :options="injectModeOptions"
              :disabled="configControlsDisabled"
              :aria-label="t('knowledge.meta.injectMode')"
              @update:model-value="onInjectModeChange"
            />
          </div>
          <div class="skill-package-config-row">
            <span class="skill-package-config-label">
              {{ t("knowledge.skill.surfaceLabel") }}
            </span>
            <BaseDropdown
              class="skill-package-dropdown"
              :model-value="skillSurfaceValue"
              :options="skillSurfaceOptions"
              :disabled="configControlsDisabled"
              :aria-label="t('knowledge.skill.surfaceLabel')"
              @update:model-value="onSkillSurfaceChange"
            />
          </div>
          <div
            v-if="showSkillCommandFields"
            class="skill-package-config-row"
          >
            <span class="skill-package-config-label">
              {{ t("knowledge.skill.commandTrigger") }}
            </span>
            <input
              v-model="skillCommandDraft"
              class="skill-package-text-input"
              type="text"
              :disabled="skillCommandInputDisabled"
              :placeholder="t('knowledge.skill.commandTriggerPlaceholder')"
              @blur="persistSkillCommandTrigger"
              @keydown="onSkillCommandKeydown"
            />
          </div>
          <div class="skill-package-config-row">
            <span class="skill-package-config-label">
              {{ t("knowledge.skillPackage.status") }}
            </span>
            <span class="skill-package-config-value">
              {{ enabledLabel }} · {{ surfaceText }}
            </span>
          </div>
        </div>
      </section>

      <section class="skill-package-section">
        <div class="skill-package-section-title">
          {{ t("knowledge.skillPackage.info") }}
        </div>
        <div class="skill-package-info-grid">
          <div
            v-for="row in infoRows"
            :key="row.label"
            class="skill-package-info-row"
          >
            <span class="skill-package-info-label">{{ row.label }}</span>
            <span class="skill-package-info-value">{{ row.value }}</span>
          </div>
        </div>
      </section>

      <section class="skill-package-section">
        <div class="skill-package-section-title">
          {{ t("knowledge.skillPackage.capabilities") }}
        </div>
        <div
          v-if="capabilityTags.length"
          class="skill-package-tags"
        >
          <span
            v-for="tag in capabilityTags"
            :key="tag"
            class="skill-package-tag"
          >
            {{ tag }}
          </span>
        </div>
        <div v-else class="skill-package-muted">
          {{ t("knowledge.skillPackage.noCapabilities") }}
        </div>
      </section>

      <section class="skill-package-section skill-package-docs-section">
        <div class="skill-package-section-heading">
          <div class="skill-package-section-title">
            {{ t("knowledge.skillPackage.documents") }}
          </div>
          <span class="skill-package-doc-count">
            {{ packageDocuments.length }}
          </span>
        </div>
        <div class="skill-package-doc-list">
          <button
            v-for="document in packageDocuments"
            :key="document.id"
            type="button"
            class="skill-package-doc-row"
            @click="emit('selectDocument', document)"
          >
            <LucideIcon
              class="skill-package-doc-icon"
              :class="documentIconClass(document)"
              :icon="documentIconNode(document)"
              :size="14"
              :stroke-width="2"
            />
            <span class="skill-package-doc-main">
              <span class="skill-package-doc-name">
                {{ documentFileName(document) }}
              </span>
              <span class="skill-package-doc-path">{{ document.path }}</span>
            </span>
            <LucideIcon
              v-if="document.commandTrigger"
              class="skill-package-command-icon"
              :icon="Terminal"
              :size="13"
              :stroke-width="2"
            />
          </button>
          <div v-if="!packageDocuments.length" class="skill-package-muted">
            {{ t("knowledge.skillPackage.noDocuments") }}
          </div>
        </div>
      </section>
    </main>
  </div>
</template>

<style scoped>
.skill-package-preview {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel-bg);
  color: var(--text-color);
  overflow: hidden;
}

.skill-package-header {
  flex-shrink: 0;
  padding: 18px 22px 16px;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 18px;
}

.skill-package-title-row {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 12px;
}

.skill-package-icon {
  width: 32px;
  height: 32px;
  border: 1px solid color-mix(in srgb, var(--accent-color) 28%, var(--border-color));
  border-radius: 8px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--accent-color);
  background: color-mix(in srgb, var(--accent-color) 10%, transparent);
  flex-shrink: 0;
}

.skill-package-title-main {
  min-width: 0;
}

.skill-package-eyebrow {
  font-size: 11px;
  font-weight: 700;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  line-height: 1.3;
}

.skill-package-title {
  margin: 2px 0 0;
  font-size: 20px;
  line-height: 1.25;
  font-weight: 700;
  color: var(--text-color);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.skill-package-path {
  max-width: 100%;
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.skill-package-header-side {
  max-width: 42%;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 8px;
  flex-shrink: 0;
}

.skill-package-header-action {
  flex-shrink: 0;
}

.skill-package-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 18px 22px 28px;
}

.skill-package-section {
  padding: 0 0 18px;
  margin: 0 0 18px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.skill-package-section:last-child {
  border-bottom: none;
  margin-bottom: 0;
}

.skill-package-section-title {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-color);
  line-height: 1.4;
}

.skill-package-section-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 10px;
}

.skill-package-description {
  margin: 8px 0 0;
  max-width: 760px;
  font-size: 13px;
  line-height: 1.65;
  color: var(--text-secondary);
}

.skill-package-config-grid {
  margin-top: 10px;
  display: grid;
  grid-template-columns: minmax(160px, 0.28fr) minmax(0, 1fr);
  gap: 8px 18px;
  max-width: 760px;
}

.skill-package-config-row {
  display: contents;
}

.skill-package-config-label,
.skill-package-config-value {
  min-width: 0;
  min-height: 32px;
  display: flex;
  align-items: center;
  font-size: 12px;
  line-height: 1.4;
}

.skill-package-config-label {
  color: var(--text-secondary);
}

.skill-package-config-value {
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  overflow-wrap: anywhere;
}

.skill-package-dropdown,
.skill-package-text-input {
  width: min(360px, 100%);
}

.skill-package-text-input {
  min-width: 0;
  height: 32px;
  padding: 0 10px;
  border: 1px solid var(--input-border, var(--border-color));
  border-radius: 6px;
  background: var(--input-bg, var(--panel-bg));
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  outline: none;
}

.skill-package-text-input:focus {
  border-color: var(--accent-color);
}

.skill-package-text-input:disabled {
  opacity: 0.62;
  cursor: not-allowed;
}

.skill-package-info-grid {
  margin-top: 10px;
  display: grid;
  grid-template-columns: minmax(160px, 0.28fr) minmax(0, 1fr);
  border-top: 1px solid color-mix(in srgb, var(--border-color) 68%, transparent);
}

.skill-package-info-row {
  display: contents;
}

.skill-package-info-label,
.skill-package-info-value {
  min-width: 0;
  padding: 9px 0;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 52%, transparent);
  font-size: 12px;
  line-height: 1.4;
}

.skill-package-info-label {
  color: var(--text-secondary);
  padding-right: 18px;
}

.skill-package-info-value {
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  overflow-wrap: anywhere;
}

.skill-package-tags {
  margin-top: 10px;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.skill-package-tag {
  padding: 3px 7px;
  border-radius: 5px;
  border: 1px solid color-mix(in srgb, var(--accent-color) 24%, var(--border-color));
  background: color-mix(in srgb, var(--accent-color) 8%, transparent);
  color: var(--text-color);
  font-size: 11px;
  font-weight: 700;
  line-height: 1.3;
}

.skill-package-muted {
  margin-top: 10px;
  font-size: 12px;
  color: var(--text-secondary);
}

.skill-package-doc-count {
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  color: var(--text-secondary);
}

.skill-package-doc-list {
  display: flex;
  flex-direction: column;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 68%, transparent);
}

.skill-package-doc-row {
  width: 100%;
  min-width: 0;
  min-height: 42px;
  padding: 7px 0;
  border: none;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 52%, transparent);
  background: transparent;
  color: var(--text-color);
  display: flex;
  align-items: center;
  gap: 10px;
  text-align: left;
  cursor: pointer;
}

.skill-package-doc-row:hover,
.skill-package-doc-row:focus-visible {
  background: color-mix(in srgb, var(--hover-bg) 68%, transparent);
  outline: none;
}

.skill-package-doc-icon {
  color: color-mix(in srgb, var(--accent-color) 46%, var(--text-secondary) 54%);
  flex-shrink: 0;
}

.skill-package-doc-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.skill-package-doc-name {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-color);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.skill-package-doc-path {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.skill-package-command-icon {
  color: var(--text-secondary);
  flex-shrink: 0;
}

@media (max-width: 720px) {
  .skill-package-header {
    flex-direction: column;
    align-items: stretch;
  }

  .skill-package-path {
    max-width: 100%;
    padding-top: 0;
  }

  .skill-package-info-grid {
    grid-template-columns: 1fr;
  }

  .skill-package-config-grid {
    grid-template-columns: 1fr;
  }

  .skill-package-config-label {
    min-height: 0;
  }

  .skill-package-info-label {
    padding-bottom: 0;
    border-bottom: none;
  }

  .skill-package-info-value {
    padding-top: 3px;
  }
}
</style>
