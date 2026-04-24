<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import type { SessionSummary } from "../../types";
import { formatShortcut, useKeyboardShortcuts } from "../../composables/useKeyboardShortcuts";

const MAX_RECENT_SESSIONS = 12;

const props = defineProps<{
  sessions: SessionSummary[];
  activeSessionId: string | null;
  streamingSessionIds?: Set<string>;
  showExpandPanelButton?: boolean;
}>();

const emit = defineEmits<{
  selectSession: [id: string];
  newChat: [];
  expandPanel: [];
}>();

const open = ref(false);
const pickerRef = ref<HTMLElement | null>(null);
const { state: shortcutState } = useKeyboardShortcuts();

const sortedSessions = computed(() =>
  [...props.sessions].sort((a, b) => b.updatedAt - a.updatedAt),
);

const recentSessions = computed(() => sortedSessions.value.slice(0, MAX_RECENT_SESSIONS));

const activeSession = computed(() =>
  props.activeSessionId
    ? props.sessions.find((session) => session.id === props.activeSessionId) ?? null
    : null,
);

const currentTitle = computed(() =>
  activeSession.value?.title || t("chat.session.newSession"),
);
const showNewButton = computed(() => props.activeSessionId !== null);
const newChatShortcutLabel = computed(() => formatShortcut(shortcutState.newChat));

function formatSessionTime(ts: number): string {
  const nowTs = Math.floor(Date.now() / 1000);
  const diff = Math.max(0, nowTs - ts);

  if (diff < 60) return t("common.timeJustNow");

  const units: Array<[number, string]> = [
    [60, "chat.session.time.minute"],
    [60 * 60, "chat.session.time.hour"],
    [60 * 60 * 24, "chat.session.time.day"],
    [60 * 60 * 24 * 7, "chat.session.time.week"],
    [60 * 60 * 24 * 30, "chat.session.time.month"],
    [60 * 60 * 24 * 365, "chat.session.time.year"],
  ];

  for (let i = units.length - 1; i >= 0; i--) {
    const [seconds, key] = units[i];
    if (diff >= seconds) {
      return t(key, Math.floor(diff / seconds));
    }
  }

  return t("common.timeJustNow");
}

function toggle() {
  open.value = !open.value;
}

function selectSession(id: string) {
  emit("selectSession", id);
  open.value = false;
}

function newChat() {
  emit("newChat");
  open.value = false;
}

function onClickOutside(event: MouseEvent) {
  if (pickerRef.value && !pickerRef.value.contains(event.target as Node)) {
    open.value = false;
  }
}

onMounted(() => document.addEventListener("click", onClickOutside));
onUnmounted(() => document.removeEventListener("click", onClickOutside));
</script>

<template>
  <div ref="pickerRef" class="session-compact-picker">
    <button
      v-if="props.showExpandPanelButton"
      type="button"
      class="session-compact-expand"
      :title="t('chat.session.expandList')"
      :aria-label="t('chat.session.expandList')"
      @click="emit('expandPanel')"
    >
      <svg viewBox="0 0 16 16" width="13" height="13" fill="currentColor" aria-hidden="true">
        <path d="M3 3.75A1.75 1.75 0 0 1 4.75 2h6.5A1.75 1.75 0 0 1 13 3.75v8.5A1.75 1.75 0 0 1 11.25 14h-6.5A1.75 1.75 0 0 1 3 12.25v-8.5Zm1.5 0v8.5c0 .14.11.25.25.25H6.5v-9H4.75a.25.25 0 0 0-.25.25Zm3.5-.25v9h3.25c.14 0 .25-.11.25-.25v-8.5a.25.25 0 0 0-.25-.25H8Z"/>
      </svg>
    </button>
    <button
      type="button"
      class="session-compact-trigger"
      :class="{ open }"
      :title="currentTitle"
      @click="toggle"
    >
      <span class="session-compact-title">{{ currentTitle }}</span>
      <svg class="session-compact-chevron" viewBox="0 0 16 16" fill="currentColor" width="10" height="10" aria-hidden="true">
        <path d="M4.427 5.427a.75.75 0 0 1 1.06-.013L8 7.867l2.513-2.453a.75.75 0 1 1 1.047 1.073l-3 2.927a.75.75 0 0 1-1.047 0l-3-2.927a.75.75 0 0 1-.013-1.06z"/>
      </svg>
    </button>
    <button
      v-if="showNewButton"
      type="button"
      class="session-compact-new"
      :title="t('chat.session.new')"
      @click="newChat"
    >
      +
    </button>

    <Transition name="session-compact-dropdown">
      <div v-if="open" class="session-compact-dropdown">
        <button
          type="button"
          class="session-compact-option"
          :class="{ active: activeSessionId === null }"
          @click="newChat"
        >
          <span class="session-compact-option-plus" aria-hidden="true">+</span>
          <span class="session-compact-option-title">{{ t("chat.session.newSession") }}</span>
          <span class="session-compact-option-shortcut">{{ newChatShortcutLabel }}</span>
        </button>
        <div class="session-compact-divider"></div>
        <div v-if="recentSessions.length === 0" class="session-compact-empty">
          {{ t("chat.session.noSessions") }}
        </div>
        <template v-else>
          <button
            v-for="session in recentSessions"
            :key="session.id"
            type="button"
            class="session-compact-option"
            :class="{
              active: session.id === activeSessionId,
              running: streamingSessionIds?.has(session.id),
            }"
            @click="selectSession(session.id)"
          >
            <span class="session-compact-option-dot"></span>
            <span class="session-compact-option-title">{{ session.title || t("chat.session.newSession") }}</span>
            <span class="session-compact-option-time">{{ formatSessionTime(session.updatedAt) }}</span>
          </button>
        </template>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.session-compact-picker {
  position: relative;
  z-index: 6;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
  min-height: 38px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--msg-assistant-bg) 82%, var(--bg-color) 18%);
}

.session-compact-trigger,
.session-compact-new,
.session-compact-expand,
.session-compact-option {
  font-family: inherit;
}

.session-compact-trigger {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  max-width: min(360px, calc(100vw - 72px));
  min-height: 26px;
  padding: 0 4px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  box-shadow: none;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.session-compact-trigger:hover,
.session-compact-trigger.open {
  background: var(--hover-bg);
  border-color: transparent;
  color: var(--text-color);
}

.session-compact-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
}

.session-compact-chevron {
  flex-shrink: 0;
  opacity: 0.5;
  transition: transform 0.15s ease;
}

.session-compact-trigger.open .session-compact-chevron {
  transform: rotate(180deg);
}

.session-compact-new,
.session-compact-expand {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 18px;
  line-height: 1;
  cursor: pointer;
  box-shadow: none;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.session-compact-new:hover,
.session-compact-new:focus-visible,
.session-compact-expand:hover,
.session-compact-expand:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
  outline: none;
}

.session-compact-expand svg {
  width: 15px;
  height: 15px;
}

.session-compact-trigger {
  order: 1;
}

.session-compact-new {
  order: 2;
}

.session-compact-expand {
  order: 3;
  margin-left: auto;
}

.session-compact-dropdown {
  position: absolute;
  left: 10px;
  top: calc(100% + 4px);
  width: min(360px, calc(100vw - 20px));
  max-height: min(360px, calc(100vh - 96px));
  overflow-y: auto;
  padding: 4px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--surface-elevated);
  box-shadow: 0 10px 28px rgba(15, 17, 21, 0.12);
}

:root[data-theme="dark"] .session-compact-dropdown {
  box-shadow: 0 14px 32px rgba(0, 0, 0, 0.34);
}

.session-compact-option {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 30px;
  padding: 4px 8px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  text-align: left;
  cursor: pointer;
  box-shadow: none;
}

.session-compact-option:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.session-compact-option.active {
  background: var(--active-bg);
  border-color: color-mix(in srgb, var(--accent-color) 18%, transparent);
  color: var(--text-color);
}

.session-compact-option-dot {
  width: 5px;
  height: 5px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 38%, transparent);
  flex-shrink: 0;
}

.session-compact-option-plus {
  width: 8px;
  flex-shrink: 0;
  color: var(--text-secondary);
  font-size: 13px;
  font-weight: 600;
  line-height: 1;
  text-align: center;
}

.session-compact-option.running .session-compact-option-dot {
  width: 6px;
  height: 6px;
  background: var(--accent-color);
  animation: session-compact-pulse 1.2s ease-in-out infinite;
}

.session-compact-option-title {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  font-weight: 500;
}

.session-compact-option-time {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.session-compact-option-shortcut {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.session-compact-empty {
  padding: 10px 8px;
  color: var(--text-secondary);
  font-size: 12px;
  text-align: center;
}

.session-compact-divider {
  height: 1px;
  margin: 4px 4px;
  background: var(--border-color);
}

.session-compact-dropdown-enter-active,
.session-compact-dropdown-leave-active {
  transition: opacity 0.12s ease, transform 0.12s ease;
}

.session-compact-dropdown-enter-from,
.session-compact-dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}

@keyframes session-compact-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.35; }
}
</style>
