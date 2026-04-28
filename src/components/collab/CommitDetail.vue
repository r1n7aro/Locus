<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { ChevronRight } from "lucide";
import type { GitCommitInfo, GitFileChange } from "../../types";
import { selectUnityAsset, openFileExternal } from "../../services/unity";
import { useProjectStore } from "../../stores/project";
import { t } from "../../i18n";
import { useHideMeta, canOpenInEditor, partitionMetaPaths } from "../../composables/useHideMeta";
import {
  getLocusManagedTagKind,
  getLocusManagedTagKindForPath,
  type LocusManagedFileLike,
} from "../../composables/locusManagedFiles";
import {
  persistStagingViewMode,
  readStoredStagingViewMode,
  type StagingViewMode,
} from "./stagingLayout";
import {
  buildStagingTreeRows,
  collectStagingFolderPaths,
} from "./stagingTree";
import LucideIcon from "../icons/LucideIcon.vue";
import {
  unityAssetIconClassForPath,
  unityFolderIconClass,
  unityAssetIconNodeForPath,
  unityFolderIconNode,
} from "../icons/unityAssetIcons";

const { hideMeta } = useHideMeta();
const projectStore = useProjectStore();
const hashCopied = ref(false);
const bodyRef = ref<HTMLElement | null>(null);
const bodyOverflow = ref(false);
const showBodyModal = ref(false);
const fileViewMode = ref<StagingViewMode>(readStoredStagingViewMode());
const collapsedFolders = ref(new Set<string>());

const commitMetaPartition = computed(() =>
  partitionMetaPaths(props.commitFiles),
);
const orphanMetaPaths = computed(() => commitMetaPartition.value.orphanMetaPaths);
const orphanMetaCount = computed(() => orphanMetaPaths.value.size);
const filteredCommitFiles = computed(() =>
  hideMeta.value
    ? props.commitFiles.filter((f) => !commitMetaPartition.value.hideableMetaPaths.has(f.path))
    : props.commitFiles,
);
const commitTreeRows = computed(() =>
  buildStagingTreeRows(filteredCommitFiles.value, collapsedFolders.value),
);
const hiddenMetaCount = computed(() => {
  if (!hideMeta.value) return 0;
  return commitMetaPartition.value.hideableMetaPaths.size;
});

const props = defineProps<{
  commit: GitCommitInfo | null;
  commitBody: string;
  commitFiles: GitFileChange[];
  filesLoading: boolean;
  detailKind: "commit" | "stash";
  detailLabel: string;
  activeFilePath: string | null;
}>();

const emit = defineEmits<{
  (e: "selectFile", file: GitFileChange): void;
}>();

const formattedDate = computed(() => {
  if (!props.commit) return "";
  const d = new Date(props.commit.date * 1000);
  const y = d.getFullYear();
  const mo = String(d.getMonth() + 1).padStart(2, "0");
  const da = String(d.getDate()).padStart(2, "0");
  const h = String(d.getHours()).padStart(2, "0");
  const mi = String(d.getMinutes()).padStart(2, "0");
  const s = String(d.getSeconds()).padStart(2, "0");
  return `${y}-${mo}-${da} ${h}:${mi}:${s}`;
});

// Stable hue from author name → pill background + contrasting text
const authorHue = computed(() => {
  const name = props.commit?.author ?? "";
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = (hash * 31 + name.charCodeAt(i)) | 0;
  }
  return Math.abs(hash) % 360;
});

const authorPillStyle = computed(() => {
  const h = authorHue.value;
  return {
    background: `hsl(${h}, 65%, 42%)`,
    color: `hsl(${h}, 85%, 94%)`,
  };
});

const shortHash = computed(() => props.commit?.hash?.slice(0, 8) ?? "");

function checkBodyOverflow() {
  const el = bodyRef.value;
  if (!el) {
    bodyOverflow.value = false;
    return;
  }
  bodyOverflow.value = el.scrollHeight - el.clientHeight > 1;
}

watch(
  () => props.commitBody,
  () => {
    nextTick(checkBodyOverflow);
  },
  { immediate: true },
);
watch(filteredCommitFiles, (files) => {
  const validPaths = collectStagingFolderPaths(files);
  if (collapsedFolders.value.size === 0) return;
  const next = new Set([...collapsedFolders.value].filter((path) => validPaths.has(path)));
  if (next.size !== collapsedFolders.value.size) {
    collapsedFolders.value = next;
  }
});
watch(fileViewMode, (mode) => {
  persistStagingViewMode(mode);
});

function openBodyModal() {
  if (!props.commitBody || !bodyOverflow.value) return;
  showBodyModal.value = true;
}

async function copyHash() {
  if (!props.commit?.hash) return;
  try {
    await navigator.clipboard.writeText(props.commit.hash);
    hashCopied.value = true;
    setTimeout(() => { hashCopied.value = false; }, 1500);
  } catch (e) {
    // ignore
  }
}

function fileStatusLabel(status: string): string {
  switch (status) {
    case "M": return "M";
    case "A": return "A";
    case "D": return "D";
    case "R": return "R";
    case "?": return "U";
    default: return status;
  }
}

function fileStatusClass(status: string): string {
  switch (status) {
    case "M": return "status-modified";
    case "A": case "?": return "status-added";
    case "D": return "status-deleted";
    case "R": return "status-renamed";
    default: return "status-modified";
  }
}

function fileName(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1];
}

function fileDir(path: string): string {
  const parts = path.split("/");
  if (parts.length <= 1) return "";
  return parts.slice(0, -1).join("/") + "/";
}

function locusBadgeLabel(file: LocusManagedFileLike): string | null {
  const kind = getLocusManagedTagKind(file);
  return kind ? t(`collab.locusTag.${kind}`) : null;
}

function folderLocusBadgeLabel(path: string): string | null {
  const kind = getLocusManagedTagKindForPath(path);
  return kind ? t(`collab.locusTag.${kind}`) : null;
}

function fileTreeIconClass(path: string) {
  return unityAssetIconClassForPath(path, { isFolder: false });
}

function treeIndentPx(depth: number) {
  if (depth <= 0) return 12;
  return 12 + depth * 20;
}

function toggleFileViewMode(mode: StagingViewMode) {
  fileViewMode.value = mode;
}

function toggleTreeFolder(chainPaths: readonly string[], expanded: boolean) {
  const next = new Set(collapsedFolders.value);
  if (expanded) {
    const collapsedPath = chainPaths[chainPaths.length - 1];
    if (collapsedPath) next.add(collapsedPath);
  } else {
    for (const path of chainPaths) {
      next.delete(path);
    }
  }
  collapsedFolders.value = next;
}

</script>

<template>
  <div class="files-panel">
    <div class="files-scroll">
      <!-- Commit header -->
      <div class="commit-detail-card">
        <div class="commit-detail-header">
          <div class="commit-detail-subject-row">
            <span class="commit-detail-subject ui-select-text" :title="commit?.message || ''">{{ commit?.message }}</span>
            <div
              v-if="commit?.hash"
              class="commit-detail-hash-group"
            >
              <span class="commit-detail-hash-text ui-select-text" :title="commit.hash">#{{ shortHash }}</span>
              <button
                class="commit-detail-hash-copy ui-select-none"
                :class="{ copied: hashCopied }"
                :title="`${commit.hash}\n${t('common.clickToCopy')}`"
                @click="copyHash"
              >
                <svg class="commit-detail-hash-icon" viewBox="0 0 16 16" width="11" height="11" fill="currentColor" aria-hidden="true">
                  <path v-if="!hashCopied" d="M5 2a2 2 0 0 0-2 2v8h1.5V4A.5.5 0 0 1 5 3.5h6V2H5zm3 3a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h5a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2H8zm-.5 2a.5.5 0 0 1 .5-.5h5a.5.5 0 0 1 .5.5v7a.5.5 0 0 1-.5.5H8a.5.5 0 0 1-.5-.5V7z"/>
                  <path v-else d="M13.78 4.22a.75.75 0 0 1 0 1.06l-7 7a.75.75 0 0 1-1.06 0l-3.5-3.5a.75.75 0 1 1 1.06-1.06L6.25 10.69l6.47-6.47a.75.75 0 0 1 1.06 0z"/>
                </svg>
              </button>
            </div>
            <span
              v-if="commit?.author"
              class="commit-detail-author"
              :style="authorPillStyle"
              :title="`${commit.author}\n${formattedDate}`"
            >{{ commit.author }}</span>
          </div>
        </div>

        <!-- Description: auto height, clamped at max, click to expand -->
        <div class="commit-detail-meta" :class="{ 'commit-detail-meta-empty': !commitBody }">
          <div class="commit-detail-desc-label">Description</div>
          <div
            ref="bodyRef"
            class="commit-detail-body ui-select-text"
            :class="{
              'commit-detail-body-empty': !commitBody,
              'commit-detail-body-clickable': bodyOverflow,
            }"
            :title="bodyOverflow ? t('collab.clickToViewFull') : ''"
            @click="openBodyModal"
          >
            {{ commitBody || t("collab.noDescription") }}
          </div>
        </div>
      </div>

      <!-- Full description modal -->
      <Teleport to="body">
        <div
          v-if="showBodyModal"
          class="commit-body-modal-backdrop"
          @click.self="showBodyModal = false"
        >
          <div class="commit-body-modal">
            <div class="commit-body-modal-header">
              <span class="commit-body-modal-title ui-select-text">{{ commit?.message }}</span>
              <button class="commit-body-modal-close ui-select-none" @click="showBodyModal = false">&times;</button>
            </div>
            <div class="commit-body-modal-content ui-select-text">{{ commitBody }}</div>
          </div>
        </div>
      </Teleport>

      <!-- Separator + file changes header -->
      <div class="files-top-header">
        <div class="files-change-count">
          <span class="change-number">{{ filteredCommitFiles.length }}</span>
          <span class="change-label">{{ detailKind === "stash" ? "stash contents" : "changes introduced by" }}</span>
          <span v-if="hiddenMetaCount > 0" class="files-change-hidden">({{ t("collab.hiddenMetaInline", hiddenMetaCount) }})</span>
          <span class="change-branch ui-select-text">{{ detailLabel }}</span>
        </div>
        <div class="header-actions">
          <button
            class="view-toggle-btn"
            :class="{ active: fileViewMode === 'tree' }"
            :aria-pressed="fileViewMode === 'tree'"
            :title="fileViewMode === 'tree' ? t('collab.view.list') : t('collab.view.tree')"
            @click="toggleFileViewMode(fileViewMode === 'tree' ? 'list' : 'tree')"
          >
            <svg v-if="fileViewMode === 'tree'" viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true">
              <path d="M2.75 3a.75.75 0 0 0 0 1.5h10.5a.75.75 0 0 0 0-1.5H2.75zm0 4.25a.75.75 0 0 0 0 1.5h10.5a.75.75 0 0 0 0-1.5H2.75zm0 4.25a.75.75 0 0 0 0 1.5h10.5a.75.75 0 0 0 0-1.5H2.75z"/>
            </svg>
            <svg v-else viewBox="0 0 16 16" width="14" height="14" fill="none" aria-hidden="true">
              <path d="M3 3.5a1 1 0 1 1 2 0 1 1 0 0 1-2 0zm8.25 0a.75.75 0 0 0 0 1.5h2a.75.75 0 0 0 0-1.5h-2zM5 4.25h2.5v3H11a1.75 1.75 0 0 1 1.75 1.75v1.75h.5a.75.75 0 0 1 0 1.5h-2a.75.75 0 0 1 0-1.5h.5V9A.25.25 0 0 0 11 8.75H7.5v2A1.75 1.75 0 0 1 5.75 12.5h-.5a1 1 0 1 1 0-1.5h.5a.25.25 0 0 0 .25-.25v-6.5H5z" fill="currentColor"/>
            </svg>
          </button>
          <button
            class="hide-meta-btn ui-select-none"
            :class="{ active: hideMeta }"
            @click="hideMeta = !hideMeta"
            :title="t('common.hideMeta')"
          >.meta</button>
        </div>
      </div>
      <div v-if="orphanMetaCount > 0" class="files-section-warning files-top-warning">
        {{ t("collab.orphanMetaWarning", orphanMetaCount) }}
      </div>

      <div v-if="filesLoading" class="files-loading">{{ t("common.loading") }}</div>
      <div v-else-if="filteredCommitFiles.length === 0" class="files-empty">{{ t("collab.noFileChanges") }}</div>
      <div v-else class="file-list" :class="{ 'staging-tree-list': fileViewMode === 'tree' }">
        <template v-if="fileViewMode === 'tree'">
          <div
            v-for="row in commitTreeRows"
            :key="row.key"
          >
            <div v-if="row.kind === 'folder'" class="staging-tree-row staging-tree-folder-row">
              <button
                type="button"
                class="staging-tree-folder-btn"
                :style="{ paddingLeft: `${treeIndentPx(row.depth)}px` }"
                :title="row.path"
                :aria-label="row.expanded ? t('merge.tree.toggleCollapse', row.name) : t('merge.tree.toggleExpand', row.name)"
                @click="toggleTreeFolder(row.chainPaths, row.expanded)"
              >
                <span class="staging-tree-branch" :class="{ open: row.expanded }" aria-hidden="true">
                  <LucideIcon class="staging-tree-chevron" :icon="ChevronRight" :size="10" />
                </span>
                <span
                  class="staging-tree-folder-icon"
                  :class="[{ open: row.expanded }, unityFolderIconClass(row.expanded)]"
                  aria-hidden="true"
                >
                  <LucideIcon :icon="unityFolderIconNode(row.expanded)" :size="13" />
                </span>
                <span class="staging-tree-folder-name">{{ row.name }}</span>
                <span v-if="folderLocusBadgeLabel(row.path)" class="locus-badge ui-select-none">{{ folderLocusBadgeLabel(row.path) }}</span>
              </button>
            </div>

            <div
              v-else
              class="file-item staging-tree-file-main"
              :class="{ selected: props.activeFilePath === row.file.path }"
              :style="{ paddingLeft: `${treeIndentPx(row.depth)}px` }"
              :title="row.file.path"
              @click="emit('selectFile', row.file)"
            >
              <span class="file-status ui-select-none" :class="fileStatusClass(row.file.status)">{{ fileStatusLabel(row.file.status) }}</span>
              <LucideIcon
                class="staging-tree-file-icon"
                :class="fileTreeIconClass(row.file.path)"
                :icon="unityAssetIconNodeForPath(row.file.path, { isFolder: false })"
                :size="14"
              />
              <span class="staging-file-copy">
                <span class="file-name ui-select-text">{{ fileName(row.file.path) }}</span>
                <span v-if="locusBadgeLabel(row.file)" class="locus-badge ui-select-none">{{ locusBadgeLabel(row.file) }}</span>
                <span v-if="row.file.lfs" class="lfs-badge ui-select-none">LFS</span>
                <span v-if="orphanMetaPaths.has(row.file.path)" class="orphan-meta-badge ui-select-none" :title="t('collab.orphanMetaHint')">{{ t("collab.orphanMetaTag") }}</span>
              </span>
              <button v-if="projectStore.unityConnected" class="file-action-btn unity-btn ui-select-none" @click.stop="selectUnityAsset(row.file.path)" :title="t('common.selectInUnity')">
                <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M6.4 1L1 8l5.4 7h3.2L6.2 9.5H15v-3H6.2L9.6 1H6.4z"/></svg>
              </button>
              <button v-if="canOpenInEditor(row.file.path)" class="file-action-btn open-btn ui-select-none" @click.stop="openFileExternal(row.file.path)" :title="t('common.openInEditor')">
                <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M8 1C4.1 1 1 4.1 1 8s3.1 7 7 7 7-3.1 7-7-3.1-7-7-7zm0 12.5c-3 0-5.5-2.5-5.5-5.5S5 2.5 8 2.5s5.5 2.5 5.5 5.5-2.5 5.5-5.5 5.5zM6 5l6 3-6 3V5z"/></svg>
              </button>
            </div>
          </div>
        </template>

        <template v-else>
          <div
            v-for="f in filteredCommitFiles"
            :key="f.path"
            class="file-item"
            :class="{ selected: props.activeFilePath === f.path }"
            :title="f.path"
            @click="emit('selectFile', f)"
          >
            <span class="file-status ui-select-none" :class="fileStatusClass(f.status)">{{ fileStatusLabel(f.status) }}</span>
            <span class="file-name ui-select-text">{{ fileName(f.path) }}</span>
            <span v-if="locusBadgeLabel(f)" class="locus-badge ui-select-none">{{ locusBadgeLabel(f) }}</span>
            <span v-if="f.lfs" class="lfs-badge ui-select-none">LFS</span>
            <span v-if="orphanMetaPaths.has(f.path)" class="orphan-meta-badge ui-select-none" :title="t('collab.orphanMetaHint')">{{ t("collab.orphanMetaTag") }}</span>
            <span class="file-dir ui-select-text">{{ fileDir(f.path) }}</span>
            <button v-if="projectStore.unityConnected" class="file-action-btn unity-btn ui-select-none" @click.stop="selectUnityAsset(f.path)" :title="t('common.selectInUnity')">
              <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M6.4 1L1 8l5.4 7h3.2L6.2 9.5H15v-3H6.2L9.6 1H6.4z"/></svg>
            </button>
            <button v-if="canOpenInEditor(f.path)" class="file-action-btn open-btn ui-select-none" @click.stop="openFileExternal(f.path)" :title="t('common.openInEditor')">
              <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M8 1C4.1 1 1 4.1 1 8s3.1 7 7 7 7-3.1 7-7-3.1-7-7-7zm0 12.5c-3 0-5.5-2.5-5.5-5.5S5 2.5 8 2.5s5.5 2.5 5.5 5.5-2.5 5.5-5.5 5.5zM6 5l6 3-6 3V5z"/></svg>
            </button>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>
