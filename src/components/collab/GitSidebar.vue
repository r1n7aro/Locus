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
  Search,
  Settings,
  Tag,
  Tags,
} from "lucide";
import type { GitBranchInfo, GitBranchTarget, GitGraphRef, GitHistoryTarget, GitRemoteBranch, GitStashEntry, GitSubmoduleInfo } from "../../types";
import { t } from "../../i18n";
import LucideIcon from "../icons/LucideIcon.vue";
import BaseButton from "../ui/BaseButton.vue";

const props = defineProps<{
  localBranches: GitBranchInfo[];
  remoteBranches: [string, GitRemoteBranch[]][];
  stashes: GitStashEntry[];
  unanchoredStashHashes: Set<string>;
  tags: GitGraphRef[];
  submodules: GitSubmoduleInfo[];
  selectedHistoryHash: string | null;
  toolbarTarget?: HTMLElement | null;
  expandLocal: boolean;
  expandRemotes: boolean;
  expandedRemoteNames: Set<string>;
  expandStashes: boolean;
  expandTags: boolean;
  expandSubmodules: boolean;
}>();

const emit = defineEmits<{
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
  <div class="git-sidebar">
    <Teleport v-if="props.toolbarTarget" :to="props.toolbarTarget">
      <BaseButton class="sidebar-toolbar-button" :title="t('collab.search.open')" :aria-label="t('collab.search.open')" @click="emit('openSearch', $event)">
        <LucideIcon :icon="Search" :size="14" />
      </BaseButton>
      <BaseButton class="sidebar-toolbar-button" :title="t('git.config.open')" :aria-label="t('git.config.open')" @click="emit('openGitConfig', $event)">
        <LucideIcon :icon="Settings" :size="14" />
      </BaseButton>
    </Teleport>
    <div class="sidebar-scroll">

      <!-- LOCAL -->
      <div class="sidebar-section">
        <button type="button" class="sidebar-section-header" :aria-expanded="props.expandLocal" @click="emit('toggleLocal')">
          <LucideIcon class="chevron" :class="{ expanded: props.expandLocal }" :icon="ChevronRight" :size="11" />
          <LucideIcon class="section-icon" :icon="GitBranch" :size="14" />
          <span class="section-label">{{ t("collab.sidebar.localBranches") }}</span>
          <span class="section-count">{{ props.localBranches.length }}</span>
        </button>
        <div v-if="props.expandLocal" class="sidebar-section-body">
          <button type="button"
            v-for="b in props.localBranches" :key="b.name"
            class="sidebar-item branch-item" :class="{ active: isSelectedBranch(b) || (!props.selectedHistoryHash && b.isCurrent) }"
            :title="b.name + '\n' + b.shortHash + ' ' + b.message"
            @click="emit('selectBranch', { kind: 'localBranch', branch: b })"
            @dblclick="emit('branchDblclick', { kind: 'localBranch', branch: b })"
            @keydown.enter.prevent="emit('branchDblclick', { kind: 'localBranch', branch: b })"
            @contextmenu.prevent="emit('branchContextmenu', $event, { kind: 'localBranch', branch: b })"
          >
            <LucideIcon class="item-icon branch-icon" :icon="GitBranch" :size="12" />
            <span class="item-label">{{ b.name }}</span>
            <span v-if="b.isCurrent" class="current-badge">HEAD</span>
          </button>
          <div v-if="props.localBranches.length === 0" class="sidebar-empty">{{ t("collab.noLocalBranch") }}</div>
        </div>
      </div>

      <!-- REMOTE -->
      <div class="sidebar-section">
        <button type="button" class="sidebar-section-header" :aria-expanded="props.expandRemotes" @click="emit('toggleRemotes')">
          <LucideIcon class="chevron" :class="{ expanded: props.expandRemotes }" :icon="ChevronRight" :size="11" />
          <LucideIcon class="section-icon" :icon="Globe" :size="14" />
          <span class="section-label">{{ t("collab.sidebar.remoteBranches") }}</span>
        </button>
        <div v-if="props.expandRemotes" class="sidebar-section-body">
          <template v-for="[remoteName, branches] in props.remoteBranches" :key="remoteName">
            <button type="button" class="sidebar-item remote-group" :aria-expanded="props.expandedRemoteNames.has(remoteName)" @click="emit('toggleRemoteName', remoteName)">
              <LucideIcon class="chevron small" :class="{ expanded: props.expandedRemoteNames.has(remoteName) }" :icon="ChevronRight" :size="10" />
              <LucideIcon class="item-icon" :icon="Globe" :size="12" />
              <span class="item-label">{{ remoteName }}</span>
            </button>
            <template v-if="props.expandedRemoteNames.has(remoteName)">
              <button type="button"
                v-for="rb in branches" :key="remoteName + '/' + rb.name"
                class="sidebar-item nested branch-item"
                :class="{ active: isSelectedBranch(rb) }"
                :title="remoteName + '/' + rb.name + '\n' + rb.shortHash + ' ' + rb.message"
                @click="emit('selectBranch', { kind: 'remoteBranch', remoteName, branch: rb })"
                @dblclick="emit('branchDblclick', { kind: 'remoteBranch', remoteName, branch: rb })"
                @keydown.enter.prevent="emit('branchDblclick', { kind: 'remoteBranch', remoteName, branch: rb })"
                @contextmenu.prevent="emit('branchContextmenu', $event, { kind: 'remoteBranch', remoteName, branch: rb })"
              >
                <LucideIcon class="item-icon branch-icon" :icon="GitBranch" :size="12" />
                <span class="item-label">{{ rb.name }}</span>
              </button>
            </template>
          </template>
          <div v-if="props.remoteBranches.length === 0" class="sidebar-empty">{{ t("collab.noRemoteBranch") }}</div>
        </div>
      </div>

      <!-- STASHES -->
      <div class="sidebar-section">
        <button type="button" class="sidebar-section-header" :aria-expanded="props.expandStashes" @click="emit('toggleStashes')">
          <LucideIcon class="chevron" :class="{ expanded: props.expandStashes }" :icon="ChevronRight" :size="11" />
          <LucideIcon class="section-icon" :icon="Archive" :size="14" />
          <span class="section-label">{{ t("collab.sidebar.stashes") }}</span>
          <span v-if="props.stashes.length > 0" class="section-count">{{ props.stashes.length }}</span>
        </button>
        <div v-if="props.expandStashes" class="sidebar-section-body">
          <button type="button"
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
          </button>
          <div v-if="props.stashes.length === 0" class="sidebar-empty">{{ t("collab.noStash") }}</div>
        </div>
      </div>

      <!-- TAGS -->
      <div v-if="props.tags.length > 0" class="sidebar-section">
        <button type="button" class="sidebar-section-header" :aria-expanded="props.expandTags" @click="emit('toggleTags')">
          <LucideIcon class="chevron" :class="{ expanded: props.expandTags }" :icon="ChevronRight" :size="11" />
          <LucideIcon class="section-icon" :icon="Tags" :size="14" />
          <span class="section-label">{{ t("collab.sidebar.tags") }}</span>
          <span class="section-count">{{ props.tags.length }}</span>
        </button>
        <div v-if="props.expandTags" class="sidebar-section-body">
          <button type="button"
            v-for="tag in props.tags" :key="tag.fullName"
            class="sidebar-item tag-item"
            :class="{ active: props.selectedHistoryHash === tag.targetHash }"
            :title="tag.shortName + ' @ ' + tag.targetHash.slice(0, 7)"
            @click="emit('selectTag', tag)"
          >
            <LucideIcon class="item-icon tag-icon" :icon="Tag" :size="12" />
            <span class="item-label">{{ tag.shortName }}</span>
          </button>
        </div>
      </div>

      <!-- SUBMODULES -->
      <div v-if="props.submodules.length > 0" class="sidebar-section">
        <button type="button" class="sidebar-section-header" :aria-expanded="props.expandSubmodules" @click="emit('toggleSubmodules')">
          <LucideIcon class="chevron" :class="{ expanded: props.expandSubmodules }" :icon="ChevronRight" :size="11" />
          <LucideIcon class="section-icon" :icon="FolderGit2" :size="14" />
          <span class="section-label">{{ t("collab.sidebar.submodules") }}</span>
          <span v-if="props.submodules.length > 0" class="section-count">{{ props.submodules.length }}</span>
        </button>
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
  </div>
</template>

<style scoped>
.git-sidebar {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  background: var(--sidebar-bg);
}
.sidebar-toolbar-button {
  width: 26px;
  min-width: 26px;
  height: 26px;
  min-height: 26px;
  padding: 0;
  border-color: transparent;
}
.sidebar-scroll {
  flex: 1;
  min-height: 0;
  padding: 4px 0;
  overflow: auto;
}
.sidebar-section + .sidebar-section {
  margin-top: 6px;
}
.sidebar-section-header,
.sidebar-item {
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  min-width: 0;
  min-height: 30px;
  padding: 2px 10px;
  border: none;
  background: transparent;
  color: color-mix(in srgb, var(--text-color) 78%, var(--text-secondary) 22%);
  font-family: var(--font-ui);
  font-size: 12px;
  text-align: left;
  cursor: pointer;
  overflow: hidden;
  transition: background 0.1s ease;
}
.sidebar-section-header {
  font-weight: 600;
  color: var(--text-secondary);
}
.sidebar-item {
  padding-left: 28px;
}
.sidebar-item.nested {
  padding-left: 46px;
}
.sidebar-section-header:hover,
.sidebar-item:hover {
  background: var(--hover-bg);
}
.sidebar-item.active {
  background: var(--active-bg);
}
.sidebar-section-header:focus-visible,
.sidebar-item:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}
.chevron,
.section-icon,
.item-icon,
.submodule-status {
  flex-shrink: 0;
  color: var(--text-secondary);
}
.chevron {
  width: 12px;
  transition: transform 0.15s ease;
}
.chevron.expanded {
  transform: rotate(90deg);
}
.section-label,
.item-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.section-count {
  font-size: 11px;
  font-weight: 400;
  color: var(--text-secondary);
}
.current-badge,
.stash-state-tag {
  flex-shrink: 0;
  padding: 0 4px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  font-size: 10px;
  line-height: 16px;
  color: var(--text-secondary);
}
.submodule-status {
  display: flex;
  align-items: center;
}
.sub-ok {
  color: var(--status-good-fg);
}
.sub-modified {
  color: var(--status-warn-fg);
}
.sidebar-empty {
  padding: 6px 28px;
  font-size: 12px;
  color: var(--text-secondary);
}
</style>
