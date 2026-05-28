<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type {
  KnowledgeCatalogStats,
  KnowledgeDocumentSummary,
  KnowledgeDocumentType,
  InjectedPromptItem,
} from "../../types";
import { t } from "../../i18n";
import { listAgentInjectedItems } from "../../services/agent";
import { useAgentStore } from "../../stores/agent";
import type { ExplorerNode } from "../../composables/useKnowledgeState";
import BaseButton from "../ui/BaseButton.vue";

const UNITY_REFERENCE_MANAGED_DIR = "unity-official-docs";

const props = defineProps<{
  stats: KnowledgeCatalogStats | null;
  loading: boolean;
  activeType: KnowledgeDocumentType;
  documents: KnowledgeDocumentSummary[];
  directoryCount: number;
  tree: ExplorerNode[];
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "createExternalFolder", source?: "feishu" | "unity"): void;
}>();

function typeLabel(type: KnowledgeDocumentType): string {
  switch (type) {
    case "design":
      return t("knowledge.type.design");
    case "memory":
      return t("knowledge.type.memory");
    case "skill":
      return t("knowledge.type.skill");
    case "reference":
      return t("knowledge.type.reference");
  }
}

function subtitleForType(type: KnowledgeDocumentType): string {
  switch (type) {
    case "design":
      return t("knowledge.dashboard.design.subtitle");
    case "memory":
      return t("knowledge.dashboard.memoryType.subtitle");
    case "skill":
      return t("knowledge.dashboard.skillType.subtitle");
    case "reference":
      return t("knowledge.dashboard.reference.subtitle");
  }
}

function formatDateTime(value: number): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleString(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const countFormatter = new Intl.NumberFormat("zh-CN");

function formatTokenCount(value: number): string {
  return t(
    "knowledge.injectionPreview.tokenCount",
    countFormatter.format(value),
  );
}

function formatByteSize(value: number): string {
  if (!value) return "0 B";
  if (value < 1024) return `${countFormatter.format(value)} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 * 1024 * 1024)
    return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function normalizeRelativePath(path: string): string {
  return path
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
}

function isUnityReferenceDocumentPath(path: string): boolean {
  const normalized = normalizeRelativePath(path);
  return (
    normalized === UNITY_REFERENCE_MANAGED_DIR ||
    normalized.startsWith(`${UNITY_REFERENCE_MANAGED_DIR}/`)
  );
}

function treeHasUnityReferenceFolder(nodes: ExplorerNode[]): boolean {
  for (const node of nodes) {
    if (node.kind !== "folder") continue;
    if (
      normalizeRelativePath(node.relativePath) === UNITY_REFERENCE_MANAGED_DIR
    ) {
      return true;
    }
    if (treeHasUnityReferenceFolder(node.children)) return true;
  }
  return false;
}

function estimateTextTokens(text: string): number {
  if (!text) return 0;
  return Math.ceil(text.length / 4);
}

function promptTypeLabel(type: KnowledgeDocumentType): string {
  switch (type) {
    case "design":
      return "design/ :: Project design direction discussed with the user";
    case "memory":
      return "memory/ :: All of your memory";
    case "skill":
      return "skill/ :: Standard workflows for getting work done";
    case "reference":
      return "reference/ :: External material";
  }
}

function promptFileName(document: KnowledgeDocumentSummary): string {
  const normalizedPath = document.path.replace(/\\/g, "/");
  return normalizedPath.split("/").pop() || document.title;
}

function promptFileDesc(document: KnowledgeDocumentSummary): string {
  return document.title.trim() || promptFileName(document);
}

function renderFallbackTreeLines(
  nodes: ExplorerNode[],
  showFiles: boolean,
  maxVisibleFiles: number,
): string[] {
  const folderNodes = nodes.filter(
    (node): node is Extract<ExplorerNode, { kind: "folder" }> =>
      node.kind === "folder",
  );
  const packageNodes = nodes.filter(
    (node): node is Extract<ExplorerNode, { kind: "package" }> =>
      node.kind === "package",
  );
  const documentNodes = nodes.filter(
    (node): node is Extract<ExplorerNode, { kind: "document" }> =>
      node.kind === "document",
  );
  const entries: Array<{ label: string; nested: string[] }> = [];

  for (const folder of folderNodes) {
    entries.push({
      label: `${folder.name}/`,
      nested: renderFallbackTreeLines(
        folder.children,
        showFiles,
        maxVisibleFiles,
      ),
    });
  }

  for (const packageNode of packageNodes) {
    entries.push({
      label: `${packageNode.name}/`,
      nested: renderFallbackTreeLines(
        packageNode.children,
        showFiles,
        maxVisibleFiles,
      ),
    });
  }

  if (showFiles) {
    for (const documentNode of documentNodes.slice(0, maxVisibleFiles)) {
      entries.push({
        label: `${promptFileName(documentNode.document)} :: ${promptFileDesc(documentNode.document)}`,
        nested: [],
      });
    }
    const hiddenCount = Math.max(0, documentNodes.length - maxVisibleFiles);
    if (hiddenCount > 0) {
      entries.push({
        label: `<${hiddenCount} files hidden>`,
        nested: [],
      });
    }
  } else if (documentNodes.length > 0) {
    entries.push({
      label: `<${documentNodes.length} files hidden>`,
      nested: [],
    });
  }

  if (!entries.length) return ["└─ <empty>"];

  return entries.flatMap((entry, index) => {
    const isLast = index === entries.length - 1;
    const branch = isLast ? "└─ " : "├─ ";
    const childPrefix = isLast ? "   " : "│  ";
    return [
      `${branch}${entry.label}`,
      ...entry.nested.map((line) => `${childPrefix}${line}`),
    ];
  });
}

function fallbackAlwaysOnText(
  type: KnowledgeDocumentType,
  tree: ExplorerNode[],
): string {
  const showFiles = type !== "reference";
  const maxVisibleFiles = type === "design" ? 2 : 3;
  const lines = [
    "### Structure",
    "",
    "```tree",
    "knowledge/",
    `└─ ${promptTypeLabel(type)}`,
    ...renderFallbackTreeLines(tree, showFiles, maxVisibleFiles).map(
      (line) => `   ${line}`,
    ),
    "```",
  ];
  return lines.join("\n");
}

function extractKnowledgeStructureBlock(content: string): string {
  const normalized = content.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  const start = lines.findIndex((line) => line.trim() === "### Structure");
  if (start < 0) return "";
  const rest = lines.slice(start + 1);
  const endOffset = rest.findIndex((line) => /^###\s+/.test(line.trim()));
  const sectionLines =
    endOffset >= 0
      ? lines.slice(start, start + 1 + endOffset)
      : lines.slice(start);
  return sectionLines.join("\n").trim();
}

function extractKnowledgeNamedBlock(content: string, heading: string): string {
  const normalized = content.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  const start = lines.findIndex((line) => line.trim() === heading);
  if (start < 0) return "";
  const rest = lines.slice(start + 1);
  const endOffset = rest.findIndex(
    (line) => /^###\s+/.test(line.trim()) || /^##\s+/.test(line.trim()),
  );
  const sectionLines =
    endOffset >= 0
      ? lines.slice(start, start + 1 + endOffset)
      : lines.slice(start);
  return sectionLines.join("\n").trim();
}

function extractL2FullDocumentBlock(
  content: string,
  type: KnowledgeDocumentType,
): string {
  const section =
    extractKnowledgeNamedBlock(content, "### L2 Full Documents") ||
    extractKnowledgeNamedBlock(content, "### L2 Memory");
  if (!section) return "";

  const lines = section.replace(/\r\n/g, "\n").split("\n");
  const blocks: string[] = [];
  for (let index = 0; index < lines.length; index += 1) {
    if (lines[index].trim().startsWith(`#### ${type}/`)) {
      const blockLines = [lines[index]];
      for (let next = index + 1; next < lines.length; next += 1) {
        if (/^####\s+/.test(lines[next].trim())) break;
        blockLines.push(lines[next]);
      }
      blocks.push(blockLines.join("\n").trim());
    }
  }
  return blocks.join("\n\n");
}

function extractStructureBranch(
  structureSection: string,
  type: KnowledgeDocumentType,
): string {
  if (!structureSection) return "";
  const normalized = structureSection.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  const branchPattern = new RegExp(`^[├└]─\\s+${type}/`);
  const start = lines.findIndex((line) => branchPattern.test(line));
  if (start < 0) return "";

  const branchLines = [lines[start]];
  for (let index = start + 1; index < lines.length; index += 1) {
    const line = lines[index];
    if (/^[├└]─\s+/.test(line)) break;
    if (line.trim() === "```") break;
    branchLines.push(line);
  }
  return branchLines.join("\n").trim();
}

function findKnowledgeContext(items: InjectedPromptItem[]): string {
  return items.find((item) => item.id === "knowledge_context")?.content ?? "";
}

function isKnowledgeRuleItem(item: InjectedPromptItem): boolean {
  return item.id.startsWith("knowledge_rule::");
}

function injectedItemDocType(
  item: InjectedPromptItem,
): KnowledgeDocumentType | null {
  const docType = (item.meta as { docType?: unknown } | null | undefined)
    ?.docType;
  return docType === "design" ||
    docType === "memory" ||
    docType === "skill" ||
    docType === "reference"
    ? docType
    : null;
}

const agentStore = useAgentStore();
const injectedItems = ref<InjectedPromptItem[]>([]);
const knowledgeContext = computed(() =>
  findKnowledgeContext(injectedItems.value),
);

async function loadKnowledgeContext() {
  const agentId = agentStore.selectedAgentId.trim();
  if (!agentId) {
    injectedItems.value = [];
    return;
  }
  try {
    injectedItems.value = await listAgentInjectedItems(agentId);
  } catch {
    injectedItems.value = [];
  }
}

watch(
  () =>
    `${agentStore.selectedAgentId}::${props.documents.length}::${props.directoryCount}`,
  () => {
    void loadKnowledgeContext();
  },
  { immediate: true },
);

const activeDocuments = computed(() =>
  props.documents
    .filter((doc) => doc.type === props.activeType)
    .sort((left, right) => right.updatedAt - left.updatedAt),
);

const totalDocuments = computed(() => activeDocuments.value.length);
const hasUnityReferenceDocs = computed(
  () =>
    props.documents.some(
      (doc) =>
        doc.type === "reference" && isUnityReferenceDocumentPath(doc.path),
    ) || treeHasUnityReferenceFolder(props.tree ?? []),
);
const showUnityImportHint = computed(
  () => props.activeType === "reference" && !hasUnityReferenceDocs.value,
);
const totalBytes = computed(() =>
  activeDocuments.value.reduce((sum, doc) => sum + (doc.byteSize ?? 0), 0),
);
const pathCount = computed(
  () => activeDocuments.value.filter((doc) => doc.injectMode === "path").length,
);
const excerptCount = computed(
  () =>
    activeDocuments.value.filter((doc) => doc.injectMode === "excerpt").length,
);
const fullCount = computed(
  () => activeDocuments.value.filter((doc) => doc.injectMode === "full").length,
);
const ruleCount = computed(
  () => activeDocuments.value.filter((doc) => doc.injectMode === "rule").length,
);
const noneCount = computed(
  () => activeDocuments.value.filter((doc) => doc.injectMode === "none").length,
);
const autoMaintainedCount = computed(
  () =>
    activeDocuments.value.filter((doc) => !doc.readOnly && doc.aiMaintained)
      .length,
);
const proposalMaintainedCount = computed(
  () =>
    activeDocuments.value.filter((doc) => !doc.readOnly && !doc.aiMaintained)
      .length,
);
const readOnlyCount = computed(
  () => activeDocuments.value.filter((doc) => doc.readOnly).length,
);
const lexicalOnlyCount = computed(
  () =>
    activeDocuments.value.filter(
      (doc) => !!doc.lexicalSearchEnabled && !doc.semanticSearchEnabled,
    ).length,
);
const semanticOnlyCount = computed(
  () =>
    activeDocuments.value.filter(
      (doc) => !doc.lexicalSearchEnabled && !!doc.semanticSearchEnabled,
    ).length,
);
const hybridRetrievalCount = computed(
  () =>
    activeDocuments.value.filter(
      (doc) => !!doc.lexicalSearchEnabled && !!doc.semanticSearchEnabled,
    ).length,
);
const retrievalDisabledCount = computed(
  () =>
    activeDocuments.value.filter(
      (doc) => !doc.lexicalSearchEnabled && !doc.semanticSearchEnabled,
    ).length,
);
const maintenanceItems = computed(() => [
  {
    label: t("knowledge.dashboard.maintenance.auto"),
    value: autoMaintainedCount.value,
  },
  {
    label: t("knowledge.dashboard.maintenance.proposal"),
    value: proposalMaintainedCount.value,
  },
  {
    label: t("knowledge.dashboard.maintenance.readOnly"),
    value: readOnlyCount.value,
  },
]);
const retrievalItems = computed(() => [
  {
    label: t("knowledge.dashboard.retrieval.lexicalOnly"),
    value: lexicalOnlyCount.value,
  },
  {
    label: t("knowledge.dashboard.retrieval.semanticOnly"),
    value: semanticOnlyCount.value,
  },
  {
    label: t("knowledge.dashboard.retrieval.hybrid"),
    value: hybridRetrievalCount.value,
  },
  {
    label: t("knowledge.dashboard.retrieval.disabled"),
    value: retrievalDisabledCount.value,
  },
]);
const injectModeItems = computed(() => [
  {
    label: t("knowledge.meta.inject.none"),
    value: noneCount.value,
  },
  {
    label: t("knowledge.meta.inject.path"),
    value: pathCount.value,
  },
  {
    label: t("knowledge.meta.inject.excerpt"),
    value: excerptCount.value,
  },
  {
    label: t("knowledge.meta.inject.full"),
    value: fullCount.value,
  },
  {
    label: t("knowledge.meta.inject.rule"),
    value: ruleCount.value,
  },
]);
const recentDocuments = computed(() => activeDocuments.value.slice(0, 8));
const activeTree = computed(() => props.tree ?? []);
const fallbackAlwaysOnTokens = computed(() =>
  estimateTextTokens(fallbackAlwaysOnText(props.activeType, activeTree.value)),
);
const structureTokenEstimate = computed(() => {
  const structureSection = extractKnowledgeStructureBlock(
    knowledgeContext.value,
  );
  const typeBranch = extractStructureBranch(structureSection, props.activeType);
  return typeBranch
    ? estimateTextTokens(typeBranch)
    : fallbackAlwaysOnTokens.value;
});
const l2FullDocumentTokenEstimate = computed(() =>
  estimateTextTokens(
    extractL2FullDocumentBlock(knowledgeContext.value, props.activeType),
  ),
);
const l3RuleTokenEstimate = computed(() =>
  injectedItems.value
    .filter(
      (item) =>
        isKnowledgeRuleItem(item) &&
        injectedItemDocType(item) === props.activeType,
    )
    .reduce((total, item) => total + estimateTextTokens(item.content), 0),
);
const alwaysOnTokenEstimate = computed(
  () =>
    structureTokenEstimate.value +
    l2FullDocumentTokenEstimate.value +
    l3RuleTokenEstimate.value,
);
const tokenBreakdownItems = computed(() => {
  const items = [
    {
      label: t("knowledge.dashboard.structure"),
      value: structureTokenEstimate.value,
    },
  ];
  if (l2FullDocumentTokenEstimate.value > 0) {
    items.push({
      label: t("knowledge.dashboard.l2FullDocuments"),
      value: l2FullDocumentTokenEstimate.value,
    });
  }
  items.push(
    {
      label: t("knowledge.dashboard.l3Rules"),
      value: l3RuleTokenEstimate.value,
    },
    {
      label: t("knowledge.dashboard.totalTokenUsage"),
      value: alwaysOnTokenEstimate.value,
    },
  );
  return items;
});

const overviewMeta = computed(() => {
  const total = props.stats?.total ?? props.documents.length;
  if (!total) return "";
  return t("knowledge.dashboard.meta", total);
});
</script>

<template>
  <div class="overview-panel">
    <div class="overview-header">
      <div class="overview-header-main">
        <div class="overview-title-row">
          <span class="overview-title">{{ typeLabel(activeType) }}</span>
          <span v-if="overviewMeta" class="overview-title-meta"
            >· {{ overviewMeta }}</span
          >
        </div>
        <div class="overview-subtitle">{{ subtitleForType(activeType) }}</div>
      </div>
      <button
        type="button"
        class="overview-close-btn"
        :aria-label="t('common.close')"
        :title="t('common.close')"
        @click="emit('close')"
      >
        &times;
      </button>
    </div>

    <div v-if="loading && !documents.length" class="overview-loading">
      {{ t("common.loading") }}
    </div>

    <template v-else>
      <div
        class="overview-grid overview-grid-top"
        :class="{ 'overview-grid-default': true }"
      >
        <div class="overview-left-stack">
          <section
            class="overview-card overview-card-primary overview-card-span-two"
          >
            <div class="card-title">
              {{ t("knowledge.dashboard.documents") }}
            </div>
            <div class="summary-metric-grid">
              <div class="summary-metric">
                <span class="summary-metric-label">{{
                  t("knowledge.overview.documentsUnit")
                }}</span>
                <span class="summary-metric-value">{{ totalDocuments }}</span>
              </div>
              <div class="summary-metric">
                <span class="summary-metric-label">{{
                  t("knowledge.dashboard.totalSize")
                }}</span>
                <span class="summary-metric-value">{{
                  formatByteSize(totalBytes)
                }}</span>
              </div>
            </div>
          </section>

          <section
            class="overview-card overview-card-mode overview-card-span-two"
          >
            <div class="overview-mode-stack">
              <section class="overview-mode-section">
                <div class="card-title">
                  {{ t("knowledge.dashboard.maintenance") }}
                </div>
                <div class="overview-mode-list">
                  <div
                    v-for="item in maintenanceItems"
                    :key="item.label"
                    class="overview-mode-row detail-row"
                  >
                    <span class="detail-key">{{ item.label }}</span>
                    <span class="overview-mode-value">{{ item.value }}</span>
                  </div>
                </div>
              </section>

              <section class="overview-mode-section">
                <div class="card-title">
                  {{ t("knowledge.dashboard.retrieval") }}
                </div>
                <div class="overview-mode-list">
                  <div
                    v-for="item in retrievalItems"
                    :key="item.label"
                    class="overview-mode-row detail-row"
                  >
                    <span class="detail-key">{{ item.label }}</span>
                    <span class="overview-mode-value">{{ item.value }}</span>
                  </div>
                </div>
              </section>

              <section class="overview-mode-section">
                <div class="card-title">
                  {{ t("knowledge.dashboard.injectMode") }}
                </div>
                <div class="overview-mode-list">
                  <div
                    v-for="item in injectModeItems"
                    :key="item.label"
                    class="overview-mode-row detail-row"
                  >
                    <span class="detail-key">{{ item.label }}</span>
                    <span class="overview-mode-value">{{ item.value }}</span>
                  </div>
                </div>
              </section>
            </div>
          </section>
        </div>

        <div class="overview-right-stack">
          <section
            v-if="activeType !== 'reference'"
            class="overview-card overview-card-token"
          >
            <div class="card-title">
              {{ t("knowledge.dashboard.tokenUsage") }}
            </div>
            <div class="inject-token-row">
              <span class="inject-token-label">{{
                t("knowledge.dashboard.alwaysOnTokenUsage")
              }}</span>
              <span class="inject-token-value">{{
                formatTokenCount(alwaysOnTokenEstimate)
              }}</span>
            </div>
            <div class="stats-section overview-primary-footer">
              <div class="stats-grid stats-grid-three">
                <div
                  v-for="item in tokenBreakdownItems"
                  :key="item.label"
                  class="stats-cell"
                >
                  <span class="stats-label">{{ item.label }}</span>
                  <span class="stats-value">{{
                    formatTokenCount(item.value)
                  }}</span>
                </div>
              </div>
            </div>
          </section>

          <section v-else class="overview-card overview-card-note">
            <div class="card-title-row">
              <span class="card-title">{{
                t("knowledge.directoryConfig.panel.external")
              }}</span>
              <BaseButton
                class="overview-card-action"
                size="sm"
                @click="emit('createExternalFolder')"
              >
                {{ t("knowledge.referenceFolder.external.createAction") }}
              </BaseButton>
            </div>
            <div class="overview-note-copy">
              {{ t("knowledge.referenceFolder.external.overviewHint") }}
            </div>
            <div v-if="showUnityImportHint" class="overview-note-action-row">
              <span class="overview-note-emphasis">
                {{ t("knowledge.referenceFolder.external.unityOverviewHint") }}
              </span>
              <BaseButton
                class="overview-note-action"
                size="sm"
                @click="emit('createExternalFolder', 'unity')"
              >
                {{ t("knowledge.referenceFolder.external.importUnityAction") }}
              </BaseButton>
            </div>
          </section>

          <section class="overview-card overview-card-recent">
            <div class="card-title-row">
              <span class="card-title">{{
                t("knowledge.dashboard.recent")
              }}</span>
            </div>
            <div v-if="recentDocuments.length" class="recent-list">
              <div
                v-for="document in recentDocuments"
                :key="document.id"
                class="recent-row"
                :class="{ 'recent-row-with-time': document.updatedAt > 0 }"
              >
                <span class="recent-main">
                  <span class="recent-title">{{ document.title }}</span>
                  <span class="recent-path">{{ document.path }}</span>
                </span>
                <span v-if="document.updatedAt > 0" class="recent-time">{{
                  formatDateTime(document.updatedAt)
                }}</span>
              </div>
            </div>
            <div v-else class="section-empty">
              {{ t("knowledge.overview.empty") }}
            </div>
          </section>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.overview-panel {
  flex: 1;
  padding: 16px 20px 20px;
  overflow: auto;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
  container-type: inline-size;
}

.overview-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 14px;
}

.overview-header-main {
  min-width: 0;
  flex: 1;
}

.overview-title-row {
  display: flex;
  align-items: baseline;
  gap: 6px;
  margin-bottom: 4px;
}

.overview-title {
  font-size: 18px;
  line-height: 1.2;
  font-weight: 600;
  color: var(--text-color);
}

.overview-title-meta {
  font-size: 12px;
  color: var(--text-secondary);
}

.overview-subtitle {
  max-width: 760px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.overview-close-btn {
  width: 24px;
  height: 24px;
  border-radius: 4px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 16px;
  line-height: 1;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  flex-shrink: 0;
}

.overview-close-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.overview-loading,
.section-empty {
  min-height: 120px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.overview-grid {
  display: grid;
  gap: 12px;
}

.overview-grid-top {
  grid-template-columns: minmax(340px, 0.86fr) minmax(300px, 1.14fr);
  align-items: start;
  margin-bottom: 12px;
}

.overview-grid-default {
  grid-template-areas:
    "documents token"
    "mode recent";
  align-items: stretch;
}

.overview-left-stack {
  min-width: 0;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
  align-content: start;
}

.overview-card-span-two {
  grid-column: 1 / -1;
}

.overview-grid-default > .overview-left-stack,
.overview-grid-default > .overview-right-stack {
  display: contents;
}

.overview-grid-default > .overview-left-stack > .overview-card-span-two {
  grid-column: auto;
}

.overview-grid-default > .overview-left-stack > .overview-card-primary {
  grid-area: documents;
}

.overview-grid-default > .overview-left-stack > .overview-card-mode {
  grid-area: mode;
}

.overview-grid-default > .overview-right-stack > .overview-card-token {
  grid-area: token;
}

.overview-grid-default > .overview-right-stack > .overview-card-recent {
  grid-area: recent;
}

.overview-grid-default > .overview-left-stack > .overview-card-primary,
.overview-grid-default > .overview-left-stack > .overview-card-mode,
.overview-grid-default > .overview-right-stack > .overview-card-token,
.overview-grid-default > .overview-right-stack > .overview-card-recent {
  height: 100%;
}

.overview-right-stack {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.overview-card {
  min-width: 0;
  padding: 14px 16px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--panel-bg);
  display: flex;
  flex-direction: column;
}

.overview-card-primary {
  min-height: 0;
  justify-content: flex-start;
}

.overview-card-mode,
.overview-card-token {
  min-height: 0;
}

.card-title,
.card-title-row .card-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.card-title {
  margin-bottom: 12px;
}

.overview-primary-main {
  min-width: 0;
}

.card-title-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}

.overview-card-action {
  flex-shrink: 0;
  margin-left: auto;
}

.overview-hero-line {
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-bottom: 14px;
}

.hero-value {
  font-size: 34px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
}

.hero-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.summary-metric-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.summary-metric {
  min-width: 0;
  padding: 10px 11px;
  border: 1px solid color-mix(in srgb, var(--border-color) 80%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--input-bg) 26%);
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.summary-metric-label {
  font-size: 11px;
  line-height: 1.35;
  color: var(--text-secondary);
}

.summary-metric-value {
  font-size: 17px;
  line-height: 1.2;
  font-weight: 700;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
}

.summary-metric-value-secondary {
  font-size: 12px;
  line-height: 1.45;
  font-weight: 600;
  color: var(--text-color);
  word-break: break-word;
}

.overview-mode-stack {
  display: flex;
  flex-direction: column;
}

.overview-mode-section {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.overview-mode-section + .overview-mode-section {
  margin-top: 14px;
  padding-top: 14px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
}

.overview-mode-section .card-title {
  margin-bottom: 0;
}

.overview-mode-list {
  display: flex;
  flex-direction: column;
}

.overview-mode-row {
  padding: 6px 0;
  border-bottom: 1px solid
    color-mix(in srgb, var(--border-color) 70%, transparent);
}

.overview-mode-row:last-child {
  border-bottom: none;
  padding-bottom: 0;
}

.overview-mode-value {
  text-align: right;
  font-size: 16px;
  font-weight: 700;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
}

.inject-token-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  padding-bottom: 12px;
  border-bottom: 1px solid
    color-mix(in srgb, var(--border-color) 76%, transparent);
}

.inject-token-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.inject-token-value {
  font-size: 20px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
}

.stats-section {
  padding-top: 14px;
}

.stats-section-title {
  margin-bottom: 10px;
  font-size: 11px;
  line-height: 1.4;
  font-weight: 600;
  color: var(--text-secondary);
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.stats-grid {
  display: grid;
  gap: 10px 18px;
}

.stats-grid-three {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.stats-grid-four {
  grid-template-columns: repeat(4, minmax(0, 1fr));
}

.stats-cell {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.stats-label {
  display: block;
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.stats-value {
  font-size: 18px;
  font-weight: 700;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
}

.overview-primary-footer {
  margin-top: 0;
}

.overview-card-recent {
  min-height: 220px;
}

.detail-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.detail-key {
  font-size: 12px;
  color: var(--text-secondary);
}

.detail-value {
  text-align: right;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
}

.overview-card-note {
  min-height: 0;
}

.overview-note-copy {
  font-size: 12px;
  line-height: 1.65;
  color: var(--text-secondary);
}

.overview-note-action-row {
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.overview-note-emphasis {
  min-width: 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.overview-note-action {
  flex-shrink: 0;
  margin-left: auto;
}

.recent-list {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 0;
  overflow: auto;
}

.recent-row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  width: 100%;
  padding: 8px 10px;
  border: 1px solid var(--border-color);
  border-radius: 7px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--input-bg) 24%);
  text-align: left;
}

.recent-row-with-time {
  justify-content: space-between;
}

.recent-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.recent-title {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.35;
}

.recent-path {
  font-size: 10px;
  line-height: 1.35;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.recent-time {
  flex-shrink: 0;
  padding-top: 1px;
  font-size: 10px;
  line-height: 1.3;
  color: var(--text-secondary);
}

@container (max-width: 1040px) {
  .overview-grid-reference {
    grid-template-areas:
      "summary"
      "source";
  }

  .stats-grid-three,
  .stats-grid-four {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@container (max-width: 720px) {
  .overview-grid-top {
    grid-template-columns: minmax(0, 1fr);
    align-items: start;
  }

  .overview-grid-default {
    grid-template-areas:
      "documents"
      "token"
      "mode"
      "recent";
    align-items: start;
  }

  .overview-left-stack {
    grid-template-columns: minmax(0, 1fr);
  }

  .summary-metric-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}

@container (max-width: 760px) {
  .overview-panel {
    padding: 14px;
  }

  .stats-grid-three,
  .stats-grid-four {
    grid-template-columns: minmax(0, 1fr);
  }

  .inject-token-row,
  .detail-row {
    align-items: flex-start;
    flex-direction: column;
  }

  .detail-value {
    text-align: left;
  }

  .overview-note-action-row {
    flex-direction: column;
    align-items: stretch;
  }

  .overview-card-action,
  .overview-note-action {
    width: 100%;
    margin-left: 0;
  }
}
</style>
