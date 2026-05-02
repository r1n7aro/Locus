<script setup lang="ts">
import { ref, watch } from "vue";
import {
  Archive,
  Check,
  ChevronRight,
  Circle,
  CircleDot,
  FolderGit2,
  GitBranch,
  Globe,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  Settings,
  Tag,
  Tags,
} from "lucide";
import type { GitBranchInfo, GitBranchTarget, GitGraphRef, GitHistoryTarget, GitRemoteBranch, GitStashEntry, GitSubmoduleInfo } from "../../types";
import { t } from "../../i18n";
import LucideIcon from "../icons/LucideIcon.vue";

const props = defineProps<{
  localBranches: GitBranchInfo[];
  remoteBranches: [string, GitRemoteBranch[]][];
  stashes: GitStashEntry[];
  unanchoredStashHashes: Set<string>;
  tags: GitGraphRef[];
  submodules: GitSubmoduleInfo[];
  selectedHistoryHash: string | null;
  sidebarCollapsed: boolean;
  expandLocal: boolean;
  expandRemotes: boolean;
  expandedRemoteNames: Set<string>;
  expandStashes: boolean;
  expandTags: boolean;
  expandSubmodules: boolean;
}>();

const emit = defineEmits<{
  (e: "toggleSidebar"): void;
  (e: "toggleLocal"): void;
  (e: "toggleRemotes"): void;
  (e: "toggleRemoteName", name: string): void;
  (e: "toggleStashes"): void;
  (e: "toggleTags"): void;
  (e: "toggleSubmodules"): void;
  (e: "selectStash", stash: GitStashEntry): void;
  (e: "selectTag", tag: GitGraphRef): void;
  (e: "selectBranch", target: GitBranchTarget): void;
  (e: "branchContextmenu", event: MouseEvent, target: GitBranchTarget): void;
  (e: "branchDblclick", target: GitBranchTarget): void;
  (e: "stashContextmenu", event: MouseEvent, target: GitHistoryTarget): void;
  (e: "openGitConfig", event: MouseEvent): void;
  (e: "openSearch", event: MouseEvent): void;
}>();

const selectedStashHashes = ref<Set<string>>(new Set());
const lastAnchorHash = ref<string | null>(null);

function clearStashSelection() {
  if (selectedStashHashes.value.size > 0) {
    selectedStashHashes.value = new Set();
  }
  lastAnchorHash.value = null;
}

function stashIndex(hash: string): number {
  return props.stashes.findIndex(stash => stash.hash === hash);
}

function onStashClick(stash: GitStashEntry, event: MouseEvent) {
  const hash = stash.hash;
  const idx = stashIndex(hash);
  if (idx < 0) return;

  if (event.ctrlKey || event.metaKey) {
    const next = new Set(selectedStashHashes.value);
    if (next.has(hash)) {
      next.delete(hash);
    } else {
      if (next.size === 0 && props.selectedHistoryHash && stashIndex(props.selectedHistoryHash) >= 0) {
        next.add(props.selectedHistoryHash);
      }
      next.add(hash);
    }
    selectedStashHashes.value = next;
    lastAnchorHash.value = hash;
    return;
  }

  if (event.shiftKey && lastAnchorHash.value) {
    const anchorIdx = stashIndex(lastAnchorHash.value);
    if (anchorIdx >= 0) {
      const [lo, hi] = anchorIdx <= idx ? [anchorIdx, idx] : [idx, anchorIdx];
      const next = new Set<string>();
      for (let i = lo; i <= hi; i++) {
        next.add(props.stashes[i].hash);
      }
      selectedStashHashes.value = next;
      return;
    }
  }

  clearStashSelection();
  lastAnchorHash.value = hash;
  emit("selectStash", stash);
}

function onStashContextMenu(event: MouseEvent, stash: GitStashEntry) {
  event.preventDefault();
  event.stopPropagation();

  let selected: GitStashEntry[];
  if (selectedStashHashes.value.size > 1 && selectedStashHashes.value.has(stash.hash)) {
    selected = props.stashes.filter(entry => selectedStashHashes.value.has(entry.hash));
  } else {
    clearStashSelection();
    selectedStashHashes.value = new Set([stash.hash]);
    lastAnchorHash.value = stash.hash;
    selected = [stash];
  }

  emit("stashContextmenu", event, {
    kind: "stash",
    stash,
    selectedStashes: selected,
  });
}

function isUnanchoredStash(stash: GitStashEntry): boolean {
  return props.unanchoredStashHashes.has(stash.hash);
}

function unanchoredStashTitle(): string {
  return t("collab.stash.unanchoredTooltip");
}

function isSelectedBranch(branch: GitBranchInfo | GitRemoteBranch): boolean {
  const selectedHash = props.selectedHistoryHash;
  const branchHash = branch.shortHash.trim();
  return !!selectedHash && !!branchHash && selectedHash.startsWith(branchHash);
}

watch(
  () => props.stashes,
  (list) => {
    if (selectedStashHashes.value.size === 0) return;
    const hashes = new Set(list.map(stash => stash.hash));
    const pruned = new Set([...selectedStashHashes.value].filter(hash => hashes.has(hash)));
    if (pruned.size !== selectedStashHashes.value.size) {
      selectedStashHashes.value = pruned;
    }
    if (lastAnchorHash.value && !hashes.has(lastAnchorHash.value)) {
      lastAnchorHash.value = null;
    }
  },
  { deep: true },
);

watch(
  () => props.selectedHistoryHash,
  (hash) => {
    if (!hash || stashIndex(hash) < 0) {
      clearStashSelection();
      return;
    }
    if (selectedStashHashes.value.size > 1 && !selectedStashHashes.value.has(hash)) {
      clearStashSelection();
    }
  },
);
</script>

<template>
  <!-- Expanded sidebar -->
  <div v-if="!props.sidebarCollapsed" class="git-sidebar">
    <div class="sidebar-header">
      <span class="sidebar-title">Git</span>
      <div class="sidebar-header-actions">
        <button class="sidebar-collapse-btn" type="button" @click="emit('toggleSidebar')" :title="t('collab.collapse')">
          <LucideIcon :icon="PanelLeftClose" :size="14" />
        </button>
        <button
          class="sidebar-search-btn"
          type="button"
          :title="t('collab.search.open')"
          :aria-label="t('collab.search.open')"
          @click="emit('openSearch', $event)"
        >
          <LucideIcon :icon="Search" :size="14" />
        </button>
      </div>
    </div>
    <div class="sidebar-scroll">

      <!-- LOCAL -->
      <div class="sidebar-section">
        <div class="sidebar-section-header" @click="emit('toggleLocal')">
          <LucideIcon class="chevron" :class="{ expanded: props.expandLocal }" :icon="ChevronRight" :size="11" />
          <LucideIcon class="section-icon" :icon="GitBranch" :size="14" />
          <span class="section-label">LOCAL</span>
          <span class="section-count">{{ props.localBranches.length }}</span>
        </div>
        <div v-if="props.expandLocal" class="sidebar-section-body">
          <div
            v-for="b in props.localBranches" :key="b.name"
            class="sidebar-item branch-item" :class="{ active: b.isCurrent || isSelectedBranch(b) }"
            :title="b.shortHash + ' ' + b.message"
            @click="emit('selectBranch', { kind: 'localBranch', branch: b })"
            @dblclick="emit('branchDblclick', { kind: 'localBranch', branch: b })"
            @contextmenu.prevent="emit('branchContextmenu', $event, { kind: 'localBranch', branch: b })"
          >
            <LucideIcon class="item-icon branch-icon" :icon="GitBranch" :size="12" />
            <span class="item-label">{{ b.name }}</span>
            <span v-if="b.isCurrent" class="current-badge">HEAD</span>
          </div>
          <div v-if="props.localBranches.length === 0" class="sidebar-empty">{{ t("collab.noLocalBranch") }}</div>
        </div>
      </div>

      <!-- REMOTE -->
      <div class="sidebar-section">
        <div class="sidebar-section-header" @click="emit('toggleRemotes')">
          <LucideIcon class="chevron" :class="{ expanded: props.expandRemotes }" :icon="ChevronRight" :size="11" />
          <LucideIcon class="section-icon" :icon="Globe" :size="14" />
          <span class="section-label">REMOTE</span>
        </div>
        <div v-if="props.expandRemotes" class="sidebar-section-body">
          <template v-for="[remoteName, branches] in props.remoteBranches" :key="remoteName">
            <div class="sidebar-item remote-group" @click="emit('toggleRemoteName', remoteName)">
              <LucideIcon class="chevron small" :class="{ expanded: props.expandedRemoteNames.has(remoteName) }" :icon="ChevronRight" :size="10" />
              <LucideIcon class="item-icon" :icon="Globe" :size="12" />
              <span class="item-label">{{ remoteName }}</span>
            </div>
            <template v-if="props.expandedRemoteNames.has(remoteName)">
              <div
                v-for="rb in branches" :key="remoteName + '/' + rb.name"
                class="sidebar-item nested branch-item"
                :class="{ active: isSelectedBranch(rb) }"
                :title="rb.shortHash + ' ' + rb.message"
                @click="emit('selectBranch', { kind: 'remoteBranch', remoteName, branch: rb })"
                @dblclick="emit('branchDblclick', { kind: 'remoteBranch', remoteName, branch: rb })"
                @contextmenu.prevent="emit('branchContextmenu', $event, { kind: 'remoteBranch', remoteName, branch: rb })"
              >
                <LucideIcon class="item-icon branch-icon" :icon="GitBranch" :size="12" />
                <span class="item-label">{{ rb.name }}</span>
              </div>
            </template>
          </template>
          <div v-if="props.remoteBranches.length === 0" class="sidebar-empty">{{ t("collab.noRemoteBranch") }}</div>
        </div>
      </div>

      <!-- STASHES -->
      <div class="sidebar-section">
        <div class="sidebar-section-header" @click="emit('toggleStashes')">
          <LucideIcon class="chevron" :class="{ expanded: props.expandStashes }" :icon="ChevronRight" :size="11" />
          <LucideIcon class="section-icon" :icon="Archive" :size="14" />
          <span class="section-label">STASHES</span>
          <span v-if="props.stashes.length > 0" class="section-count">{{ props.stashes.length }}</span>
        </div>
        <div v-if="props.expandStashes" class="sidebar-section-body">
          <div
            v-for="s in props.stashes" :key="s.hash"
            class="sidebar-item ui-select-none"
            :class="{ active: props.selectedHistoryHash === s.hash || selectedStashHashes.has(s.hash), 'stash-item': true }"
            :title="s.refName + ': ' + s.message"
            @click="onStashClick(s, $event)"
            @contextmenu="onStashContextMenu($event, s)"
          >
            <LucideIcon class="item-icon stash-icon" :icon="Archive" :size="12" />
            <span class="item-label stash-label">{{ s.message }}</span>
            <span
              v-if="isUnanchoredStash(s)"
              class="stash-state-tag"
              :title="unanchoredStashTitle()"
            >{{ t("collab.stash.unanchored") }}</span>
          </div>
          <div v-if="props.stashes.length === 0" class="sidebar-empty">{{ t("collab.noStash") }}</div>
        </div>
      </div>

      <!-- TAGS -->
      <div v-if="props.tags.length > 0" class="sidebar-section">
        <div class="sidebar-section-header" @click="emit('toggleTags')">
          <LucideIcon class="chevron" :class="{ expanded: props.expandTags }" :icon="ChevronRight" :size="11" />
          <LucideIcon class="section-icon" :icon="Tags" :size="14" />
          <span class="section-label">TAGS</span>
          <span class="section-count">{{ props.tags.length }}</span>
        </div>
        <div v-if="props.expandTags" class="sidebar-section-body">
          <div
            v-for="tag in props.tags" :key="tag.fullName"
            class="sidebar-item tag-item"
            :class="{ active: props.selectedHistoryHash === tag.targetHash }"
            :title="tag.shortName + ' @ ' + tag.targetHash.slice(0, 7)"
            @click="emit('selectTag', tag)"
          >
            <LucideIcon class="item-icon tag-icon" :icon="Tag" :size="12" />
            <span class="item-label">{{ tag.shortName }}</span>
          </div>
        </div>
      </div>

      <!-- SUBMODULES -->
      <div v-if="props.submodules.length > 0" class="sidebar-section">
        <div class="sidebar-section-header" @click="emit('toggleSubmodules')">
          <LucideIcon class="chevron" :class="{ expanded: props.expandSubmodules }" :icon="ChevronRight" :size="11" />
          <LucideIcon class="section-icon" :icon="FolderGit2" :size="14" />
          <span class="section-label">SUBMODULES</span>
          <span v-if="props.submodules.length > 0" class="section-count">{{ props.submodules.length }}</span>
        </div>
        <div v-if="props.expandSubmodules" class="sidebar-section-body">
          <div
            v-for="m in props.submodules" :key="m.path"
            class="sidebar-item"
            :title="m.path + ' @ ' + m.hash.slice(0, 7)"
          >
            <span class="submodule-status" :class="'sub-' + m.status">
              <LucideIcon v-if="m.status === 'ok'" :icon="Check" :size="12" />
              <LucideIcon v-else-if="m.status === 'modified'" :icon="CircleDot" :size="12" />
              <LucideIcon v-else :icon="Circle" :size="12" />
            </span>
            <LucideIcon class="item-icon" :icon="FolderGit2" :size="12" />
            <span class="item-label">{{ m.name }}</span>
          </div>
        </div>
      </div>

    </div>
    <div class="sidebar-footer">
      <button
        type="button"
        class="sidebar-config-btn"
        :title="t('git.config.open')"
        @click="emit('openGitConfig', $event)"
      >
        <LucideIcon class="sidebar-config-icon" :icon="Settings" :size="13" />
        <span>{{ t("git.config.open") }}</span>
      </button>
    </div>
  </div>

  <!-- Collapsed sidebar -->
  <div v-else class="sidebar-collapsed" @click="emit('toggleSidebar')" :title="t('collab.expand')">
    <LucideIcon :icon="PanelLeftOpen" :size="14" />
  </div>
</template>
