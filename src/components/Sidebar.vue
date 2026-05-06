
<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted } from "vue";
import { t } from "../i18n";
import { formatShortcut, useKeyboardShortcuts } from "../composables/useKeyboardShortcuts";

const props = defineProps<{
  workingDir: string;
  recentDirs: string[];
}>();

const emit = defineEmits<{
  newChat: [];
  selectDir: [path: string];
  browseDir: [];
}>();

const showDirDropdown = ref(false);
const dropdownRef = ref<HTMLElement | null>(null);
const { state: shortcutState } = useKeyboardShortcuts();
const newChatTitle = computed(() =>
  t("chat.session.newWithShortcut", formatShortcut(shortcutState.newChat)),
);

function shortDir(dir: string): string {
  if (!dir) return t("app.dir.notSet");
  const parts = dir.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : dir;
}

function parentPath(dir: string): string {
  const parts = dir.replace(/\\/g, "/").split("/").filter(Boolean);
  if (parts.length <= 1) return "";
  return parts.slice(0, -1).join("/");
}

function toggleDropdown() {
  showDirDropdown.value = !showDirDropdown.value;
}

function selectRecentDir(dir: string) {
  showDirDropdown.value = false;
  if (dir !== props.workingDir) {
    emit("selectDir", dir);
  }
}

function browseNewDir() {
  showDirDropdown.value = false;
  emit("browseDir");
}

function handleClickOutside(e: MouseEvent) {
  if (dropdownRef.value && !dropdownRef.value.contains(e.target as Node)) {
    showDirDropdown.value = false;
  }
}

onMounted(() => document.addEventListener("click", handleClickOutside, true));
onUnmounted(() => document.removeEventListener("click", handleClickOutside, true));
</script>

<template>
  <aside class="sidebar">
    <div class="sidebar-header">
      <span class="sidebar-title">Locus</span>
      <button class="new-chat-btn" @click="emit('newChat')" :title="newChatTitle">+</button>
    </div>

    <div class="workspace-selector" ref="dropdownRef">
      <button
        class="workspace-btn"
        :title="props.workingDir || t('app.dir.notSetTitle')"
        @click="toggleDropdown"
      >
        <svg class="ws-icon" viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
          <path d="M1 3.5A1.5 1.5 0 0 1 2.5 2h3.879a1.5 1.5 0 0 1 1.06.44l1.122 1.12A1.5 1.5 0 0 0 9.62 4H13.5A1.5 1.5 0 0 1 15 5.5v7a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5v-9z"/>
        </svg>
        <span class="ws-name">{{ shortDir(props.workingDir) }}</span>
        <svg class="ws-chevron" :class="{ open: showDirDropdown }" viewBox="0 0 16 16" fill="currentColor" width="10" height="10">
          <path d="M4.427 5.427a.75.75 0 0 1 1.06-.013L8 7.867l2.513-2.453a.75.75 0 1 1 1.047 1.073l-3 2.927a.75.75 0 0 1-1.047 0l-3-2.927a.75.75 0 0 1-.013-1.06z"/>
        </svg>
      </button>
      <Transition name="dropdown">
        <div v-if="showDirDropdown" class="dir-dropdown">
          <div class="dropdown-label">{{ t("app.dir.recentDirs") }}</div>
          <div
            v-for="dir in recentDirs"
            :key="dir"
            class="dir-item"
            :class="{ active: dir === props.workingDir }"
            @click="selectRecentDir(dir)"
            :title="dir"
          >
            <svg class="dir-item-icon" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
              <path d="M1 3.5A1.5 1.5 0 0 1 2.5 2h3.879a1.5 1.5 0 0 1 1.06.44l1.122 1.12A1.5 1.5 0 0 0 9.62 4H13.5A1.5 1.5 0 0 1 15 5.5v7a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5v-9z"/>
            </svg>
            <div class="dir-item-text">
              <span class="dir-item-name">{{ shortDir(dir) }}</span>
              <span class="dir-item-path">{{ parentPath(dir) }}</span>
            </div>
            <span v-if="dir === props.workingDir" class="dir-check">&#10003;</span>
          </div>
          <div v-if="recentDirs.length === 0" class="dropdown-empty">{{ t("app.dir.noRecords") }}</div>
          <div class="dropdown-divider"></div>
          <div class="dir-item browse" @click="browseNewDir">
            <svg class="dir-item-icon" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
              <path d="M8 2a.75.75 0 0 1 .75.75v4.5h4.5a.75.75 0 0 1 0 1.5h-4.5v4.5a.75.75 0 0 1-1.5 0v-4.5h-4.5a.75.75 0 0 1 0-1.5h4.5v-4.5A.75.75 0 0 1 8 2z"/>
            </svg>
            <span class="dir-item-name">{{ t("app.dir.browseOther") }}</span>
          </div>
        </div>
      </Transition>
    </div>

  </aside>
</template>

<style scoped>
.sidebar {
  width: 240px;
  min-width: 240px;
  height: 100vh;
  background: var(--sidebar-bg);
  border-right: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
}

.sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px;
  border-bottom: 1px solid var(--border-color);
}

.sidebar-title {
  font-size: 18px;
  font-weight: 700;
  letter-spacing: -0.5px;
}

.new-chat-btn {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-color);
  font-size: 18px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 0.15s;
  box-shadow: none;
  padding: 0;
}

.new-chat-btn:hover {
  background: var(--hover-bg);
}

.workspace-selector {
  position: relative;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
}

.workspace-btn {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: transparent;
  color: var(--text-color);
  font-size: 12px;
  cursor: pointer;
  transition: all 0.15s;
  box-shadow: none;
}

.workspace-btn:hover {
  background: var(--hover-bg);
  border-color: var(--text-secondary);
}

.ws-icon {
  opacity: 0.5;
  flex-shrink: 0;
}

.workspace-btn:hover .ws-icon {
  opacity: 0.8;
}

.ws-name {
  flex: 1;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 500;
}

.ws-chevron {
  opacity: 0.4;
  flex-shrink: 0;
  transition: transform 0.2s;
}

.ws-chevron.open {
  transform: rotate(180deg);
}

.dir-dropdown {
  position: absolute;
  left: 8px;
  right: 8px;
  top: calc(100% + 2px);
  background: var(--bg-color);
  border: 1px solid var(--border-color);
  border-radius: 10px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
  z-index: 100;
  padding: 4px;
  max-height: 320px;
  overflow-y: auto;
}

:root[data-theme="dark"] .dir-dropdown {
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
}

.dropdown-label {
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  padding: 6px 8px 4px;
}

.dir-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  border-radius: 6px;
  cursor: pointer;
  transition: background 0.12s;
  font-size: 12px;
  color: var(--text-color);
}

.dir-item:hover {
  background: var(--hover-bg);
}

.dir-item.active {
  background: var(--active-bg);
}

.dir-item-icon {
  opacity: 0.45;
  flex-shrink: 0;
}

.dir-item:hover .dir-item-icon {
  opacity: 0.7;
}

.dir-item-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.dir-item-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 500;
}

.dir-item-path {
  font-size: 10px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dir-check {
  font-size: 11px;
  color: var(--accent-color);
  flex-shrink: 0;
  opacity: 0.6;
}

.dropdown-empty {
  text-align: center;
  font-size: 11px;
  color: var(--text-secondary);
  padding: 8px;
}

.dropdown-divider {
  height: 1px;
  background: var(--border-color);
  margin: 4px 4px;
}

.dir-item.browse {
  color: var(--text-secondary);
}

.dir-item.browse:hover {
  color: var(--text-color);
}

.dropdown-enter-active,
.dropdown-leave-active {
  transition: opacity 0.15s ease, transform 0.15s ease;
}

.dropdown-enter-from,
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}


</style>
