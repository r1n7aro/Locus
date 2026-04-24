<script setup lang="ts">
import { ref, watch } from "vue";
import type { GitBranchInfo, GitBranchTarget, GitGraphRef, GitHistoryTarget, GitRemoteBranch, GitStashEntry, GitSubmoduleInfo } from "../../types";
import { t } from "../../i18n";

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
      <button class="sidebar-collapse-btn" @click="emit('toggleSidebar')" :title="t('collab.collapse')">
        <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
          <path d="M7.78 12.53a.75.75 0 0 1-1.06 0L2.47 8.28a.75.75 0 0 1 0-1.06l4.25-4.25a.75.75 0 0 1 1.06 1.06L4.81 7h7.44a.75.75 0 0 1 0 1.5H4.81l2.97 2.97a.75.75 0 0 1 0 1.06z"/>
        </svg>
      </button>
    </div>
    <div class="sidebar-scroll">

      <!-- LOCAL -->
      <div class="sidebar-section">
        <div class="sidebar-section-header" @click="emit('toggleLocal')">
          <span class="chevron" :class="{ expanded: props.expandLocal }">&#9654;</span>
          <svg class="section-icon" viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
            <path d="M4.75 7a.75.75 0 0 0 0 1.5h4.5a.75.75 0 0 0 0-1.5h-4.5zM5 4.75a.75.75 0 0 1 .75-.75h5.5a.75.75 0 0 1 0 1.5h-5.5A.75.75 0 0 1 5 4.75zM6.75 10a.75.75 0 0 0 0 1.5h2.5a.75.75 0 0 0 0-1.5h-2.5z"/>
          </svg>
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
            <svg class="item-icon branch-icon" viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
              <path d="M11.75 2.5a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5zm-2.25.75a2.25 2.25 0 1 1 3 2.122V6A2.5 2.5 0 0 1 10 8.5H6A1.5 1.5 0 0 0 4.5 10v1.128a2.251 2.251 0 1 1-1.5 0V5.372a2.25 2.25 0 1 1 1.5 0v1.836A3 3 0 0 1 6 7h4a1 1 0 0 0 1-1v-.628A2.25 2.25 0 0 1 9.5 3.25zM4.25 12a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5zM3.5 3.25a.75.75 0 1 1 1.5 0 .75.75 0 0 1-1.5 0z"/>
            </svg>
            <span class="item-label">{{ b.name }}</span>
            <span v-if="b.isCurrent" class="current-badge">HEAD</span>
          </div>
          <div v-if="props.localBranches.length === 0" class="sidebar-empty">{{ t("collab.noLocalBranch") }}</div>
        </div>
      </div>

      <!-- REMOTE -->
      <div class="sidebar-section">
        <div class="sidebar-section-header" @click="emit('toggleRemotes')">
          <span class="chevron" :class="{ expanded: props.expandRemotes }">&#9654;</span>
          <svg class="section-icon" viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
            <path d="M8 0a8 8 0 1 0 0 16A8 8 0 0 0 8 0zM1.5 8a6.5 6.5 0 1 1 13 0 6.5 6.5 0 0 1-13 0z"/>
            <path d="M8 1.5c-1.38 0-2.74 1.9-3.27 4.5h6.54C10.74 3.4 9.38 1.5 8 1.5zM4.55 7.5C4.52 7.66 4.5 7.83 4.5 8s.02.34.05.5h6.9c.03-.16.05-.33.05-.5s-.02-.34-.05-.5h-6.9zM4.73 10c.53 2.6 1.89 4.5 3.27 4.5s2.74-1.9 3.27-4.5H4.73z"/>
          </svg>
          <span class="section-label">REMOTE</span>
        </div>
        <div v-if="props.expandRemotes" class="sidebar-section-body">
          <template v-for="[remoteName, branches] in props.remoteBranches" :key="remoteName">
            <div class="sidebar-item remote-group" @click="emit('toggleRemoteName', remoteName)">
              <span class="chevron small" :class="{ expanded: props.expandedRemoteNames.has(remoteName) }">&#9654;</span>
              <svg class="item-icon" viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
                <path d="M8 0a8 8 0 1 0 0 16A8 8 0 0 0 8 0zM1.5 8a6.5 6.5 0 1 1 13 0 6.5 6.5 0 0 1-13 0z"/>
              </svg>
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
                <svg class="item-icon branch-icon" viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
                  <path d="M11.75 2.5a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5zm-2.25.75a2.25 2.25 0 1 1 3 2.122V6A2.5 2.5 0 0 1 10 8.5H6A1.5 1.5 0 0 0 4.5 10v1.128a2.251 2.251 0 1 1-1.5 0V5.372a2.25 2.25 0 1 1 1.5 0v1.836A3 3 0 0 1 6 7h4a1 1 0 0 0 1-1v-.628A2.25 2.25 0 0 1 9.5 3.25zM4.25 12a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5zM3.5 3.25a.75.75 0 1 1 1.5 0 .75.75 0 0 1-1.5 0z"/>
                </svg>
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
          <span class="chevron" :class="{ expanded: props.expandStashes }">&#9654;</span>
          <svg class="section-icon" viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
            <path d="M1.75 1h12.5c.966 0 1.75.784 1.75 1.75v10.5A1.75 1.75 0 0 1 14.25 15H1.75A1.75 1.75 0 0 1 0 13.25V2.75C0 1.784.784 1 1.75 1zm12.5 1.5H1.75a.25.25 0 0 0-.25.25v10.5c0 .138.112.25.25.25h12.5a.25.25 0 0 0 .25-.25V2.75a.25.25 0 0 0-.25-.25zM8 10a2 2 0 1 1 0-4 2 2 0 0 1 0 4z"/>
          </svg>
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
            <svg class="item-icon stash-icon" viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
              <path d="M1.75 1h12.5c.966 0 1.75.784 1.75 1.75v10.5A1.75 1.75 0 0 1 14.25 15H1.75A1.75 1.75 0 0 1 0 13.25V2.75C0 1.784.784 1 1.75 1z"/>
            </svg>
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
          <span class="chevron" :class="{ expanded: props.expandTags }">&#9654;</span>
          <svg class="section-icon" viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
            <path d="M1 7.775V2.75C1 1.784 1.784 1 2.75 1h5.025c.464 0 .91.184 1.238.513l5.474 5.474a1.75 1.75 0 0 1 0 2.475l-5.025 5.025a1.75 1.75 0 0 1-2.475 0L1.513 9.013A1.75 1.75 0 0 1 1 7.775zM2.75 2.5a.25.25 0 0 0-.25.25v5.025c0 .066.026.13.073.177l5.475 5.475a.25.25 0 0 0 .353 0l5.026-5.026a.25.25 0 0 0 0-.353L7.952 2.573a.25.25 0 0 0-.177-.073H2.75zM6 5a1 1 0 1 1-2 0 1 1 0 0 1 2 0z"/>
          </svg>
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
            <svg class="item-icon tag-icon" viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
              <path d="M1 7.775V2.75C1 1.784 1.784 1 2.75 1h5.025c.464 0 .91.184 1.238.513l5.474 5.474a1.75 1.75 0 0 1 0 2.475l-5.025 5.025a1.75 1.75 0 0 1-2.475 0L1.513 9.013A1.75 1.75 0 0 1 1 7.775zM2.75 2.5a.25.25 0 0 0-.25.25v5.025c0 .066.026.13.073.177l5.475 5.475a.25.25 0 0 0 .353 0l5.026-5.026a.25.25 0 0 0 0-.353L7.952 2.573a.25.25 0 0 0-.177-.073H2.75zM6 5a1 1 0 1 1-2 0 1 1 0 0 1 2 0z"/>
            </svg>
            <span class="item-label">{{ tag.shortName }}</span>
          </div>
        </div>
      </div>

      <!-- SUBMODULES -->
      <div v-if="props.submodules.length > 0" class="sidebar-section">
        <div class="sidebar-section-header" @click="emit('toggleSubmodules')">
          <span class="chevron" :class="{ expanded: props.expandSubmodules }">&#9654;</span>
          <svg class="section-icon" viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
            <path d="M0 2.75C0 1.784.784 1 1.75 1H5c.55 0 1.07.26 1.4.7l.9 1.2a.25.25 0 0 0 .2.1h6.75c.966 0 1.75.784 1.75 1.75v8.5A1.75 1.75 0 0 1 14.25 15H1.75A1.75 1.75 0 0 1 0 13.25V2.75zm1.75-.25a.25.25 0 0 0-.25.25v10.5c0 .138.112.25.25.25h12.5a.25.25 0 0 0 .25-.25v-8.5a.25.25 0 0 0-.25-.25H7.5c-.55 0-1.07-.26-1.4-.7l-.9-1.2a.25.25 0 0 0-.2-.1H1.75z"/>
          </svg>
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
              <svg v-if="m.status === 'ok'" viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
                <path d="M13.78 4.22a.75.75 0 0 1 0 1.06l-7.25 7.25a.75.75 0 0 1-1.06 0L2.22 9.28a.75.75 0 0 1 1.06-1.06L6 10.94l6.72-6.72a.75.75 0 0 1 1.06 0z"/>
              </svg>
              <svg v-else-if="m.status === 'modified'" viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
                <path d="M8 4a4 4 0 1 0 0 8 4 4 0 0 0 0-8z"/>
              </svg>
              <svg v-else viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
                <path d="M8 0a8 8 0 1 0 0 16A8 8 0 0 0 8 0zm3.28 5.78l-4.25 4.25a.75.75 0 0 1-1.06 0l-2.25-2.25a.75.75 0 1 1 1.06-1.06L6.5 8.44l3.72-3.72a.75.75 0 1 1 1.06 1.06z"/>
              </svg>
            </span>
            <svg class="item-icon" viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
              <path d="M0 2.75C0 1.784.784 1 1.75 1H5c.55 0 1.07.26 1.4.7l.9 1.2a.25.25 0 0 0 .2.1h6.75c.966 0 1.75.784 1.75 1.75v8.5A1.75 1.75 0 0 1 14.25 15H1.75A1.75 1.75 0 0 1 0 13.25V2.75z"/>
            </svg>
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
        <svg class="sidebar-config-icon" viewBox="0 0 16 16" width="13" height="13" fill="currentColor" aria-hidden="true">
          <path d="M6.32 1.1a.75.75 0 0 1 .72-.55h1.92a.75.75 0 0 1 .72.55l.3 1.05c.38.15.74.36 1.07.62l1.06-.29a.75.75 0 0 1 .85.34l.96 1.66a.75.75 0 0 1-.13.91l-.78.77c.03.2.05.41.05.63s-.02.43-.05.64l.78.76a.75.75 0 0 1 .13.91l-.96 1.66a.75.75 0 0 1-.85.34l-1.06-.29c-.33.26-.69.47-1.07.62l-.3 1.05a.75.75 0 0 1-.72.55H7.04a.75.75 0 0 1-.72-.55l-.3-1.05a4.56 4.56 0 0 1-1.07-.62l-1.06.29a.75.75 0 0 1-.85-.34l-.96-1.66a.75.75 0 0 1 .13-.91l.78-.76a4.34 4.34 0 0 1 0-1.27l-.78-.77a.75.75 0 0 1-.13-.91l.96-1.66a.75.75 0 0 1 .85-.34l1.06.29c.33-.26.69-.47 1.07-.62l.3-1.05zM8 5.25a2.75 2.75 0 1 0 0 5.5 2.75 2.75 0 0 0 0-5.5z"/>
        </svg>
        <span>{{ t("git.config.open") }}</span>
      </button>
    </div>
  </div>

  <!-- Collapsed sidebar -->
  <div v-else class="sidebar-collapsed" @click="emit('toggleSidebar')" :title="t('collab.expand')">
    <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
      <path d="M8.22 2.97a.75.75 0 0 1 1.06 0l4.25 4.25a.75.75 0 0 1 0 1.06l-4.25 4.25a.75.75 0 0 1-1.06-1.06L11.19 8.5H3.75a.75.75 0 0 1 0-1.5h7.44L8.22 4.03a.75.75 0 0 1 0-1.06z"/>
    </svg>
  </div>
</template>
