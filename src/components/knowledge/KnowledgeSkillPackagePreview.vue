<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Download, Package, RefreshCw, Terminal } from "lucide";
import { t } from "../../i18n";
import type {
  KnowledgeDocumentPatch,
  KnowledgeDocumentSummary,
  KnowledgeInjectMode,
  SkillManifest,
  SkillSurface,
} from "../../types";
import { skillSurfaceAllowsCommand } from "../../types";
import { refreshExternalSkills } from "../../services/knowledge";
import { useSkills } from "../../composables/useSkills";
import {
  deriveSkillSurface,
  effectiveSkillInjectMode,
  findSkillCommandConflict,
  isValidSkillCommandTrigger,
  normalizeSkillCommandTrigger,
  skillActivationInactive,
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
import BaseSwitch from "../ui/BaseSwitch.vue";

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

const injectModeOptions = computed(() => [
  {
    value: "none",
    label: labelForInjectMode("none", "skill"),
    hint: hintForInjectMode("none", "skill"),
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

// External skills (generic SKILL.md format discovered from agent directories
// such as ~/.claude/skills) reuse this panel in a read-only, rescanable mode.
const isExternalSkill = computed(() =>
  !!props.packageDocument.externalSource?.locator?.startsWith("external://"),
);

const externalLocatorParts = computed(() => {
  const locator = props.packageDocument.externalSource?.locator ?? "";
  const rest = locator.startsWith("external://")
    ? locator.slice("external://".length)
    : "";
  const [scope = "", provider = ""] = rest.split("/");
  return { scope, provider };
});

const manifest = computed<SkillManifest | null>(
  () =>
    skillItems.value.find(
      (item) =>
        (item.kind === "package" || item.kind === "external") &&
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
// The two activation channels behind the master switch: the command channel
// (slash trigger) and the auto channel (structure injection + model recall,
// live only when injectMode is path/excerpt).
const commandChannelOn = computed(() =>
  skillSurfaceAllowsCommand(packageSurface.value),
);
const effectiveInjectValue = computed(() =>
  effectiveSkillInjectMode(packageSurface.value, injectMode.value),
);
const autoChannelOn = computed(() => effectiveInjectValue.value !== "none");
const activationWarning = computed(
  () =>
    packageEnabled.value &&
    skillActivationInactive({
      skillEnabled: packageEnabled.value,
      skillSurface: packageSurface.value,
      injectMode: injectMode.value,
    }),
);
const surfaceText = computed(() => {
  if (!packageEnabled.value) return t("knowledge.skill.surfaceDisabled");
  if (commandChannelOn.value && autoChannelOn.value) {
    return t("knowledge.skill.surfaceBoth");
  }
  if (commandChannelOn.value) return t("knowledge.skill.surfaceCommand");
  if (autoChannelOn.value) return t("knowledge.skill.surfaceAuto");
  return t("knowledge.skill.channelsNone");
});
const updatedLabel = computed(() =>
  formatDateTime(manifest.value?.updatedAt ?? props.packageDocument.updatedAt),
);
const injectMode = computed(
  () => props.packageDocument.injectMode ?? "none",
);
const injectModeDropdownLabel = computed(() =>
  labelForInjectMode(effectiveInjectValue.value, "skill"),
);
const fallbackSkillName = computed(
  () => packageId.value || displayName.value,
);
const currentSkillCommandTrigger = computed(() =>
  normalizeSkillCommandTrigger(commandTrigger.value, fallbackSkillName.value),
);
const showSkillCommandFields = computed(() => commandChannelOn.value);
// Saving no longer disables the config controls: updates are optimistic and
// serialized upstream, so a disabled-dim flash on every toggle is avoided.
const skillCommandInputDisabled = computed(
  () => !showSkillCommandFields.value,
);

const capabilityTags = computed(() => {
  const tags: string[] = [];
  if (packageEnabled.value && commandChannelOn.value && commandTrigger.value) {
    tags.push(t("knowledge.skillPackage.command"));
  }
  if (packageEnabled.value && autoChannelOn.value) {
    tags.push(t("knowledge.skillPackage.auto"));
  }
  if (manifest.value?.hasUnity) tags.push(t("knowledge.skillPackage.unity"));
  if (manifest.value?.hasL0) tags.push("L0");
  if (manifest.value?.hasL1) tags.push("L1");
  if (manifest.value?.hasL2) tags.push("L2");
  return tags;
});

const externalScopeLabel = computed(() =>
  externalLocatorParts.value.scope === "project"
    ? t("knowledge.externalSkill.scopeProject")
    : t("knowledge.externalSkill.scopeUser"),
);

// Frontmatter fields Locus recognizes but intentionally does not apply; they
// render with a "not applied" badge so users know the declaration is inert.
const NOT_APPLIED_METADATA_KEYS = new Set([
  "allowed-tools",
  "allowedTools",
  "hooks",
  "context",
  "model",
  "agent",
]);

function formatMetadataValue(value: unknown): string {
  if (value == null) return "-";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

const externalMetadataRows = computed(() => {
  if (!isExternalSkill.value) return [];
  const raw = manifest.value?.extraMetadata;
  if (!raw || typeof raw !== "object") return [];
  return Object.entries(raw).map(([key, value]) => ({
    key,
    value: formatMetadataValue(value),
    notApplied: NOT_APPLIED_METADATA_KEYS.has(key),
  }));
});

const infoRows = computed(() => {
  const rows = [
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
  ];
  if (isExternalSkill.value) {
    rows.splice(
      1,
      0,
      {
        label: t("knowledge.externalSkill.provider"),
        value: `${externalLocatorParts.value.provider || "-"} · ${externalScopeLabel.value}`,
      },
      {
        label: t("knowledge.externalSkill.originPath"),
        value: manifest.value?.originPath?.trim() || "-",
      },
    );
  }
  return rows;
});

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

function onEnabledChange(value: boolean) {
  notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
  emit("updateConfig", { skillEnabled: value });
}

// The inject-mode dropdown drives the auto channel, so it also derives the
// surface: picking L0/L1 turns the auto side on, picking none turns it off.
function onInjectModeChange(value: string) {
  if (!["none", "path", "excerpt"].includes(value)) return;
  emit("updateConfig", {
    injectMode: value as KnowledgeInjectMode,
    inheritInjectMode: false,
    skillSurface: deriveSkillSurface(commandChannelOn.value, value !== "none"),
  });
}

function onCommandChannelChange(value: boolean) {
  notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
  emit("updateConfig", {
    skillSurface: deriveSkillSurface(value, autoChannelOn.value),
    commandTrigger: value
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
    source: manifest.value?.source ?? "app",
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

const rescanning = ref(false);

async function onRescanExternalSkills() {
  if (rescanning.value) return;
  rescanning.value = true;
  try {
    // The backend rescans the agent skill directories and emits
    // knowledge-changed, which refreshes the tree; reload manifests here so
    // this panel picks up metadata changes immediately.
    await refreshExternalSkills();
    await loadSkills({ force: true });
  } catch (cause) {
    notificationStore.addNotice(
      "error",
      cause instanceof Error ? cause.message : String(cause),
    );
  } finally {
    rescanning.value = false;
  }
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
            {{
              isExternalSkill
                ? t("knowledge.externalSkill.badge")
                : t("knowledge.skillPackage.badge")
            }}
          </div>
          <h1 class="skill-package-title">{{ displayName }}</h1>
        </div>
      </div>
      <div class="skill-package-header-side">
        <BaseButton
          v-if="isExternalSkill"
          class="skill-package-header-action"
          type="button"
          :disabled="rescanning"
          :title="t('knowledge.externalSkill.rescan')"
          @click="onRescanExternalSkills"
        >
          <LucideIcon :icon="RefreshCw" :size="13" :stroke-width="2.2" />
          <span>{{ t("knowledge.externalSkill.rescan") }}</span>
        </BaseButton>
        <BaseButton
          v-else
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
      <div v-if="isExternalSkill" class="skill-package-external-note">
        <p>{{ t("knowledge.externalSkill.readOnlyNote") }}</p>
        <p>{{ t("knowledge.externalSkill.unsupportedNote") }}</p>
      </div>

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
              {{ t("knowledge.skill.enabledLabel") }}
            </span>
            <span class="skill-package-config-value">
              <BaseSwitch
                :model-value="packageEnabled"
                :aria-label="t('knowledge.skill.enabledLabel')"
                @update:model-value="onEnabledChange"
              />
            </span>
          </div>
          <div class="skill-package-config-row">
            <span class="skill-package-config-label">
              {{ t("knowledge.meta.injectMode") }}
            </span>
            <BaseDropdown
              class="skill-package-dropdown"
              :model-value="effectiveInjectValue"
              :selected-label="injectModeDropdownLabel"
              :options="injectModeOptions"
              :aria-label="t('knowledge.meta.injectMode')"
              @update:model-value="onInjectModeChange"
            />
          </div>
          <div class="skill-package-config-row">
            <span class="skill-package-config-label">
              {{ t("knowledge.skill.commandChannelLabel") }}
            </span>
            <span class="skill-package-config-value">
              <BaseSwitch
                :model-value="commandChannelOn"
                :aria-label="t('knowledge.skill.commandChannelLabel')"
                @update:model-value="onCommandChannelChange"
              />
            </span>
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
        <div v-if="activationWarning" class="skill-package-warning">
          {{ t("knowledge.skill.activationWarning") }}
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

      <section
        v-if="externalMetadataRows.length"
        class="skill-package-section"
      >
        <div class="skill-package-section-title">
          {{ t("knowledge.externalSkill.metadata") }}
        </div>
        <div class="skill-package-info-grid">
          <div
            v-for="row in externalMetadataRows"
            :key="row.key"
            class="skill-package-info-row"
          >
            <span class="skill-package-info-label skill-package-metadata-key">
              <span>{{ row.key }}</span>
              <span
                v-if="row.notApplied"
                class="skill-package-metadata-badge"
                :title="t('knowledge.externalSkill.unsupportedNote')"
              >
                {{ t("knowledge.externalSkill.notApplied") }}
              </span>
            </span>
            <span class="skill-package-info-value skill-package-metadata-value">{{
              row.value
            }}</span>
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

.skill-package-external-note {
  margin: 0 0 18px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--accent-color) 22%, var(--border-color));
  border-radius: 8px;
  background: color-mix(in srgb, var(--accent-color) 6%, transparent);
  max-width: 760px;
}

.skill-package-external-note p {
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.skill-package-external-note p + p {
  margin-top: 4px;
}

.skill-package-warning {
  margin-top: 12px;
  padding: 9px 12px;
  border: 1px solid color-mix(in srgb, var(--warning-color, #d9a03f) 42%, var(--border-color));
  border-radius: 8px;
  background: color-mix(in srgb, var(--warning-color, #d9a03f) 9%, transparent);
  color: var(--text-color);
  font-size: 12px;
  line-height: 1.6;
  max-width: 760px;
}

.skill-package-metadata-key {
  gap: 6px;
  align-items: baseline;
}

.skill-package-metadata-badge {
  flex-shrink: 0;
  padding: 1px 5px;
  border: 1px solid color-mix(in srgb, var(--text-secondary) 34%, var(--border-color));
  border-radius: 4px;
  color: var(--text-secondary);
  font-size: 10px;
  line-height: 1.4;
  white-space: nowrap;
}

.skill-package-metadata-value {
  white-space: pre-wrap;
  word-break: break-word;
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
