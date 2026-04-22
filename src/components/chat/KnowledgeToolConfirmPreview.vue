<script setup lang="ts">
import { computed, ref, watch } from "vue";
import FileDiffViewer from "../diff/FileDiffViewer.vue";
import { diffStrings } from "../../services/diff";
import { t } from "../../i18n";
import type { FileDiffPayload, KnowledgeToolConfirmPreview } from "../../types";
import BaseButton from "../ui/BaseButton.vue";
import { titleForKnowledgeToolConfirm } from "./toolConfirmLabels";

const props = defineProps<{
  preview: KnowledgeToolConfirmPreview;
}>();

const INLINE_PREVIEW_MAX_LINES = 12;
const INLINE_PREVIEW_MAX_CHARS = 1200;

const diffPayload = ref<FileDiffPayload | null>(null);
let diffSeq = 0;
const activeDialog = ref<"document" | "structure" | null>(null);

const directoryModeLabel = computed(() =>
  t(`chat.toolConfirm.knowledge.mode.${props.preview.directoryMode}`),
);
const title = computed(() => titleForKnowledgeToolConfirm(props.preview));

const hasDocumentDiff = computed(() =>
  props.preview.operation === "edit"
  && props.preview.documentBeforeText != null
  && props.preview.documentAfterText != null,
);

const showAfterOnlyPreview = computed(() =>
  props.preview.operation === "create" && props.preview.documentAfterText != null,
);

const showBeforeOnlyPreview = computed(() =>
  props.preview.operation === "delete" && props.preview.documentBeforeText != null,
);

const hasStructurePreview = computed(() =>
  (props.preview.structureBeforePaths?.length ?? 0) > 0
  || (props.preview.structureAfterPaths?.length ?? 0) > 0,
);

type SummaryItem = {
  key: string;
  label: string;
  value: string;
  mono?: boolean;
  wide?: boolean;
};

const summaryItems = computed<SummaryItem[]>(() => {
  const items: SummaryItem[] = [
    {
      key: "path",
      label: t("chat.toolConfirm.knowledge.field.path"),
      value: props.preview.path,
      mono: true,
      wide: true,
    },
  ];

  if (props.preview.newPath) {
    items.push({
      key: "newPath",
      label: t("chat.toolConfirm.knowledge.field.newPath"),
      value: props.preview.newPath,
      mono: true,
      wide: true,
    });
  }

  items.push(
    {
      key: "directory",
      label: t("chat.toolConfirm.knowledge.field.targetDirectory"),
      value: props.preview.directoryPath,
      mono: true,
    },
    {
      key: "mode",
      label: t("chat.toolConfirm.knowledge.field.directoryMode"),
      value: directoryModeLabel.value,
    },
  );

  return items;
});

watch(
  () => ({
    operation: props.preview.operation,
    path: props.preview.path,
    before: props.preview.documentBeforeText ?? "",
    after: props.preview.documentAfterText ?? "",
  }),
  async ({ operation, path, before, after }) => {
    const seq = ++diffSeq;
    diffPayload.value = null;
    if (
      operation !== "edit"
      || props.preview.documentBeforeText == null
      || props.preview.documentAfterText == null
    ) {
      return;
    }
    try {
      const hunks = await diffStrings(before, after, 3);
      if (seq !== diffSeq) return;

      const additions = hunks.reduce(
        (sum, hunk) => sum + hunk.lines.filter((line) => line.kind === "add").length,
        0,
      );
      const deletions = hunks.reduce(
        (sum, hunk) => sum + hunk.lines.filter((line) => line.kind === "delete").length,
        0,
      );

      diffPayload.value = {
        key: `knowledge-confirm:${props.preview.path}`,
        filePath: path,
        status: "M",
        language: "markdown",
        isBinary: false,
        isLarge: false,
        contentState: { type: "normal" },
        stats: {
          additions,
          deletions,
          changedHunks: hunks.length,
        },
        previewSummary: [t("chat.toolConfirm.knowledge.diffSummary", additions, deletions)],
        text: { hunks },
      };
    } catch {
      if (seq === diffSeq) {
        diffPayload.value = null;
      }
    }
  },
  { immediate: true },
);

type TreeNode = {
  name: string;
  isDirectory: boolean;
  children: Map<string, TreeNode>;
};

function buildTree(paths: string[]): TreeNode[] {
  const roots = new Map<string, TreeNode>();

  for (const rawPath of paths) {
    const normalized = rawPath.trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
    if (!normalized) continue;
    const segments = normalized.split("/").filter(Boolean);
    let level = roots;

    segments.forEach((segment, index) => {
      const isLeaf = index === segments.length - 1;
      const isDirectory = !isLeaf || !segment.endsWith(".md");
      let node = level.get(segment);
      if (!node) {
        node = {
          name: segment,
          isDirectory,
          children: new Map<string, TreeNode>(),
        };
        level.set(segment, node);
      } else if (isDirectory) {
        node.isDirectory = true;
      }
      level = node.children;
    });
  }

  function sortNodes(nodes: Map<string, TreeNode>): TreeNode[] {
    return Array.from(nodes.values())
      .sort((left, right) => {
        if (left.isDirectory !== right.isDirectory) return left.isDirectory ? -1 : 1;
        return left.name.localeCompare(right.name, undefined, {
          sensitivity: "base",
          numeric: true,
        });
      })
      .map((node) => ({
        ...node,
        children: new Map(sortNodes(node.children).map((child) => [child.name, child])),
      }));
  }

  return sortNodes(roots);
}

function renderTree(paths: string[]): string {
  if (!paths.length) return t("chat.toolConfirm.knowledge.empty");

  const lines: string[] = [];
  const walk = (nodes: TreeNode[], depth: number) => {
    for (const node of nodes) {
      const label = node.isDirectory ? `${node.name}/` : node.name;
      lines.push(`${"  ".repeat(depth)}${label}`);
      walk(Array.from(node.children.values()), depth + 1);
    }
  };

  walk(buildTree(paths), 0);
  return lines.join("\n");
}

const beforeTreeText = computed(() => renderTree(props.preview.structureBeforePaths ?? []));
const afterTreeText = computed(() => renderTree(props.preview.structureAfterPaths ?? []));

type InlineExcerpt = {
  text: string;
  truncated: boolean;
};

type PreviewPanel = {
  key: "before" | "after";
  label: string;
  excerpt: InlineExcerpt;
};

function buildExcerpt(
  value: string | null | undefined,
  maxLines = INLINE_PREVIEW_MAX_LINES,
  maxChars = INLINE_PREVIEW_MAX_CHARS,
): InlineExcerpt {
  const normalized = (value ?? "").replace(/\r\n/g, "\n").replace(/\s+$/g, "");
  if (!normalized) {
    return {
      text: t("chat.toolConfirm.knowledge.empty"),
      truncated: false,
    };
  }

  const lines = normalized.split("\n");
  const limitedLines = lines.slice(0, maxLines);
  let excerpt = limitedLines.join("\n");
  let truncated = lines.length > maxLines;

  if (!truncated && excerpt.length > maxChars) {
    excerpt = excerpt.slice(0, maxChars).trimEnd();
    truncated = true;
  }

  if (truncated) {
    excerpt = `${excerpt.trimEnd()}\n…`;
  }

  return {
    text: excerpt,
    truncated,
  };
}

const documentPreviewPanels = computed<PreviewPanel[]>(() => {
  if (hasDocumentDiff.value) {
    return [
      {
        key: "before",
        label: t("chat.toolConfirm.knowledge.column.before"),
        excerpt: buildExcerpt(props.preview.documentBeforeText),
      },
      {
        key: "after",
        label: t("chat.toolConfirm.knowledge.column.after"),
        excerpt: buildExcerpt(props.preview.documentAfterText),
      },
    ];
  }

  if (showBeforeOnlyPreview.value) {
    return [
      {
        key: "before",
        label: t("chat.toolConfirm.knowledge.column.before"),
        excerpt: buildExcerpt(props.preview.documentBeforeText),
      },
    ];
  }

  if (showAfterOnlyPreview.value) {
    return [
      {
        key: "after",
        label: t("chat.toolConfirm.knowledge.column.after"),
        excerpt: buildExcerpt(props.preview.documentAfterText),
      },
    ];
  }

  return [];
});
const structureExcerpt = computed(() =>
  buildExcerpt(
    props.preview.structureAfterPaths?.length
      ? afterTreeText.value
      : beforeTreeText.value,
    10,
    900,
  ),
);
const dialogDocumentText = computed(() =>
  showAfterOnlyPreview.value ? (props.preview.documentAfterText ?? "") : (props.preview.documentBeforeText ?? ""),
);

const diffStatsLabel = computed(() => {
  if (!diffPayload.value) return "";
  return t(
    "chat.toolConfirm.knowledge.diffStats",
    diffPayload.value.stats.additions,
    diffPayload.value.stats.deletions,
    diffPayload.value.stats.changedHunks,
  );
});

const structureStatsLabel = computed(() => t(
  "chat.toolConfirm.knowledge.structureStats",
  props.preview.structureBeforePaths?.length ?? 0,
  props.preview.structureAfterPaths?.length ?? 0,
));

const canOpenDocumentDialog = computed(() =>
  hasDocumentDiff.value || documentPreviewPanels.value.some((panel) => panel.excerpt.truncated),
);

const documentDialogButtonLabel = computed(() => (
  hasDocumentDiff.value
    ? t("chat.toolConfirm.knowledge.open.diff")
    : t("chat.toolConfirm.knowledge.open.content")
));
</script>

<template>
  <div class="knowledge-tool-preview">
    <div class="knowledge-tool-preview-header">
      <div class="knowledge-tool-preview-title">{{ title }}</div>
    </div>

    <div class="knowledge-tool-preview-summary">
      <div
        v-for="item in summaryItems"
        :key="item.key"
        class="knowledge-tool-preview-summary-item"
        :class="{ wide: item.wide }"
      >
        <span class="preview-summary-label">{{ item.label }}</span>
        <code v-if="item.mono" class="preview-summary-value mono">{{ item.value }}</code>
        <span v-else class="preview-summary-value">{{ item.value }}</span>
      </div>
    </div>

    <div
      v-if="showAfterOnlyPreview || showBeforeOnlyPreview || hasDocumentDiff"
      class="knowledge-tool-preview-section"
    >
      <div class="preview-section-header">
        <div class="preview-section-headings">
          <div class="preview-section-label">{{ t("chat.toolConfirm.knowledge.section.document") }}</div>
          <div v-if="hasDocumentDiff && diffStatsLabel" class="preview-section-meta">{{ diffStatsLabel }}</div>
        </div>
        <BaseButton
          v-if="canOpenDocumentDialog"
          class="preview-open-btn"
          size="sm"
          @click="activeDialog = 'document'"
        >
          {{ documentDialogButtonLabel }}
        </BaseButton>
      </div>
      <div class="preview-inline-grid" :class="{ single: documentPreviewPanels.length === 1 }">
        <div
          v-for="panel in documentPreviewPanels"
          :key="panel.key"
          class="preview-inline-panel"
        >
          <div class="preview-inline-label">{{ panel.label }}</div>
          <pre class="preview-excerpt preview-inline-code">{{ panel.excerpt.text }}</pre>
        </div>
      </div>
    </div>

    <div v-if="hasStructurePreview" class="knowledge-tool-preview-section">
      <div class="preview-section-header">
        <div class="preview-section-headings">
          <div class="preview-section-label">{{ t("chat.toolConfirm.knowledge.section.structure") }}</div>
          <div class="preview-section-meta">{{ structureStatsLabel }}</div>
        </div>
        <BaseButton class="preview-open-btn" size="sm" @click="activeDialog = 'structure'">
          {{ t("chat.toolConfirm.knowledge.open.structure") }}
        </BaseButton>
      </div>
      <pre class="preview-excerpt">{{ structureExcerpt.text }}</pre>
    </div>

    <Teleport to="body">
      <div v-if="activeDialog" class="knowledge-preview-overlay" @click.self="activeDialog = null">
        <div class="knowledge-preview-dialog" role="dialog" aria-modal="true">
          <div class="knowledge-preview-dialog-header">
            <div class="knowledge-preview-dialog-title">
              {{ title }} · {{ activeDialog === "structure" ? t("chat.toolConfirm.knowledge.section.structure") : t("chat.toolConfirm.knowledge.section.document") }}
            </div>
            <BaseButton size="sm" @click="activeDialog = null">{{ t("common.close") }}</BaseButton>
          </div>

          <div class="knowledge-preview-dialog-body">
            <template v-if="activeDialog === 'document'">
              <FileDiffViewer
                v-if="hasDocumentDiff && diffPayload"
                :payload="diffPayload"
                mode="side-by-side"
                :hide-builtin-tabs="true"
              />
              <div v-else-if="hasDocumentDiff" class="preview-fallback-grid">
                <div class="preview-fallback-panel">
                  <div class="preview-fallback-label">{{ t("tool.diff.old") }}</div>
                  <pre class="preview-dialog-code">{{ preview.documentBeforeText }}</pre>
                </div>
                <div class="preview-fallback-panel">
                  <div class="preview-fallback-label">{{ t("tool.diff.new") }}</div>
                  <pre class="preview-dialog-code">{{ preview.documentAfterText }}</pre>
                </div>
              </div>
              <pre v-else class="preview-dialog-code">{{ dialogDocumentText }}</pre>
            </template>

            <div v-else class="preview-structure-grid">
              <div class="preview-structure-panel">
                <div class="preview-fallback-label">{{ t("tool.diff.old") }}</div>
                <pre class="preview-dialog-code">{{ beforeTreeText }}</pre>
              </div>
              <div class="preview-structure-panel">
                <div class="preview-fallback-label">{{ t("tool.diff.new") }}</div>
                <pre class="preview-dialog-code">{{ afterTreeText }}</pre>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.knowledge-tool-preview {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.knowledge-tool-preview-header {
  padding-bottom: 8px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
}

.knowledge-tool-preview-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
}

.knowledge-tool-preview-summary {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px 14px;
}

.knowledge-tool-preview-summary-item {
  min-width: 0;
  display: grid;
  gap: 3px;
  padding-bottom: 6px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 68%, transparent);
}

.knowledge-tool-preview-summary-item.wide {
  grid-column: 1 / -1;
}

.preview-summary-label {
  font-size: 11px;
  color: var(--text-secondary);
}

.preview-summary-value {
  min-width: 0;
  font-size: 12px;
  color: var(--text-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.preview-summary-value.mono {
  font-family: var(--font-mono-identifier);
}

.knowledge-tool-preview-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--bg-color) 72%, var(--panel-bg) 28%);
}

.preview-section-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.preview-section-headings {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.preview-section-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.preview-section-meta {
  font-size: 11px;
  color: var(--text-secondary);
}

.preview-open-btn {
  flex: none;
}

.preview-excerpt,
.preview-dialog-code {
  margin: 0;
  overflow: auto;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--sidebar-bg) 14%);
  color: var(--text-color);
  font-size: 12px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: var(--font-mono-block);
}

.preview-excerpt {
  max-height: 220px;
}

.preview-inline-grid,
.preview-fallback-grid,
.preview-structure-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
  min-height: 0;
}

.preview-inline-grid.single {
  grid-template-columns: minmax(0, 1fr);
}

.preview-inline-panel,
.preview-fallback-panel,
.preview-structure-panel {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.preview-inline-label,
.preview-fallback-label {
  font-size: 11px;
  color: var(--text-secondary);
}

.preview-inline-code {
  max-height: 220px;
}

.knowledge-preview-overlay {
  position: fixed;
  inset: 0;
  z-index: 220;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: color-mix(in srgb, var(--bg-color) 26%, transparent);
  backdrop-filter: blur(4px);
}

.knowledge-preview-dialog {
  width: min(1120px, calc(100vw - 48px));
  max-height: min(82vh, 920px);
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--sidebar-bg);
  overflow: hidden;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.28);
}

.knowledge-preview-dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border-color);
}

.knowledge-preview-dialog-title {
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.knowledge-preview-dialog-body {
  flex: 1;
  overflow: auto;
  padding: 14px;
}

@media (max-width: 960px) {
  .knowledge-tool-preview-summary,
  .preview-fallback-grid,
  .preview-structure-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .knowledge-preview-dialog {
    width: calc(100vw - 24px);
    max-height: calc(100vh - 24px);
  }
}
</style>
