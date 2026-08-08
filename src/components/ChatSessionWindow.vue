<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../i18n";
import { useAppBootstrap } from "../composables/useAppBootstrap";
import { normalizeAppError } from "../services/errors";
import {
  CHAT_SESSION_WINDOW_EVENT,
  chatSessionWindowKind,
  getChatSessionWindowPayload,
  type ChatSessionWindowPayload,
} from "../services/chatSessionWindow";
import { getSubWindowClaimedQuery } from "../services/subWindow";
import {
  canStartWindowDragFromTarget,
  hasTauriWindowRuntime,
  startCurrentWindowDragging,
} from "../services/tauriRuntime";
import { useChatStore } from "../stores/chat";
import { useUiStore } from "../stores/ui";
import ChatWorkspaceView from "./ChatWorkspaceView.vue";
import SessionCompactPicker from "./chat/SessionCompactPicker.vue";
import TopBannerHost from "./TopBannerHost.vue";

const chatStore = useChatStore();
chatStore.setActiveSessionSelectionPersistence(false);
const uiStore = useUiStore();
const { bootstrapCritical, registerListeners, cleanup } = useAppBootstrap({
  syncActiveSessionSelection: false,
  handleExternalScriptOpen: false,
});

const payload = ref<ChatSessionWindowPayload>(getChatSessionWindowPayload());
const bootstrapped = ref(false);
const bootstrapError = ref("");
let unlistenPayload: UnlistenFn | null = null;

const activeSession = computed(() =>
  chatStore.sessions.find((session) => session.id === chatStore.activeSessionId) ?? null,
);
const sessionTitle = computed(() =>
  activeSession.value?.title?.trim()
  || payload.value.title?.trim()
  || payload.value.sessionId
  || t("chat.session.newSession"),
);

async function selectPayloadSession(nextPayload: ChatSessionWindowPayload) {
  const sessionId = nextPayload.sessionId.trim();
  if (!sessionId || !bootstrapped.value) return;
  if (!chatStore.sessions.some((session) => session.id === sessionId)) {
    await chatStore.refreshSessions();
  }
  if (!chatStore.sessions.some((session) => session.id === sessionId)) {
    throw new Error(t("chat.session.windowUnavailable"));
  }
  await chatStore.selectSession(sessionId, { persist: false });
}

function applyPayload(nextPayload: ChatSessionWindowPayload) {
  const sessionId = nextPayload.sessionId?.trim() || "";
  if (!sessionId && !nextPayload.newChat) return;
  payload.value = {
    sessionId,
    title: nextPayload.title?.trim() || undefined,
    newChat: nextPayload.newChat === true,
  };
  if (bootstrapped.value) {
    if (payload.value.newChat) {
      createWindowSession();
    } else {
      void selectPayloadSession(payload.value).catch((cause) => {
        bootstrapError.value = normalizeAppError(cause).message;
      });
    }
  }
}

function handleTitlebarPointerDown(event: PointerEvent) {
  if (event.button !== 0 || event.detail > 1) return;
  if (!canStartWindowDragFromTarget(event.target)) return;
  event.preventDefault();
  startCurrentWindowDragging();
}

function selectWindowSession(sessionId: string) {
  bootstrapError.value = "";
  const session = chatStore.sessions.find((candidate) => candidate.id === sessionId);
  payload.value = {
    sessionId,
    title: session?.title?.trim() || undefined,
    newChat: false,
  };
  void chatStore.selectSession(sessionId, { persist: false });
}

function createWindowSession() {
  bootstrapError.value = "";
  payload.value = {
    sessionId: "",
    newChat: true,
  };
  chatStore.newChat({ persistSelection: false });
}

watch(sessionTitle, (title) => {
  if (!hasTauriWindowRuntime()) return;
  void getCurrentWindow().setTitle(`Locus - ${title}`).catch(() => {});
}, { immediate: true });

onMounted(async () => {
  try {
    unlistenPayload = await listen<ChatSessionWindowPayload>(
      CHAT_SESSION_WINDOW_EVENT,
      (event) => applyPayload(event.payload),
    );

    const initialSessionId = payload.value.sessionId.trim();
    if (initialSessionId) {
      const latestQuery = await getSubWindowClaimedQuery(
        chatSessionWindowKind(initialSessionId),
      ).catch(() => null);
      if (latestQuery) {
        applyPayload(getChatSessionWindowPayload(`?${latestQuery}`));
      }
    }

    await bootstrapCritical();
    await registerListeners();
    bootstrapped.value = true;
    if (payload.value.newChat) {
      createWindowSession();
    } else {
      await selectPayloadSession(payload.value);
    }
  } catch (cause) {
    bootstrapError.value = normalizeAppError(cause).message;
  }
});

onUnmounted(() => {
  unlistenPayload?.();
  unlistenPayload = null;
  cleanup();
});
</script>

<template>
  <main class="chat-session-window-root">
    <header
      class="chat-session-window-titlebar"
      data-tauri-drag-region
      @pointerdown="handleTitlebarPointerDown"
    >
      <SessionCompactPicker
        class="chat-session-window-picker"
        :sessions="chatStore.sessions"
        :active-session-id="chatStore.activeSessionId"
        :streaming-session-ids="chatStore.streamingSessionIds"
        :show-views="false"
        @select-session="selectWindowSession"
        @new-chat="createWindowSession"
      />
      <div class="chat-session-window-drag-region" data-tauri-drag-region></div>
      <div class="chat-session-window-controls" data-window-no-drag>
        <button
          type="button"
          class="chat-session-window-control"
          :title="t('app.win.minimize')"
          @click="uiStore.winMinimize"
        >
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="1" y="5.5" width="10" height="1" fill="currentColor" />
          </svg>
        </button>
        <button
          type="button"
          class="chat-session-window-control"
          :title="t('app.win.maximize')"
          @click="uiStore.winToggleMaximize"
        >
          <svg v-if="!uiStore.isMaximized" viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="1.5" y="1.5" width="9" height="9" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
          </svg>
          <svg v-else viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <rect x="2.5" y="0.5" width="8" height="8" rx="1" fill="none" stroke="currentColor" stroke-width="1.1" />
            <rect x="0.5" y="2.5" width="8" height="8" rx="1" fill="var(--sidebar-bg)" stroke="currentColor" stroke-width="1.1" />
          </svg>
        </button>
        <button
          type="button"
          class="chat-session-window-control is-close"
          :title="t('app.win.close')"
          @click="uiStore.winClose"
        >
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </header>

    <TopBannerHost />
    <div v-if="bootstrapError" class="chat-session-window-state is-error">
      {{ bootstrapError }}
    </div>
    <div v-else-if="!bootstrapped" class="chat-session-window-state">
      {{ t("common.loading") }}
    </div>
    <ChatWorkspaceView
      v-else
      class="chat-session-window-workspace"
      active
      layout-mode="auto"
      :show-session-navigation="false"
      :persist-session-selection="false"
      session-panel-storage-scope="standalone-session"
    />
  </main>
</template>

<style scoped>
.chat-session-window-root {
  width: 100vw;
  height: 100vh;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  color: var(--text-color);
  border: 1px solid var(--border-strong);
}

.chat-session-window-titlebar {
  -webkit-app-region: drag;
  position: relative;
  z-index: 120;
  height: 38px;
  min-height: 38px;
  display: flex;
  align-items: center;
  width: 100%;
  min-width: 0;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
}

.chat-session-window-picker {
  -webkit-app-region: no-drag;
  flex: 0 1 460px;
  width: auto;
  min-width: 0;
  max-width: 460px;
  margin-left: 16px;
}

:deep(.chat-session-window-picker.session-compact-picker) {
  width: 100%;
  min-height: 37px;
  height: 37px;
  padding: 4px 7px;
  border-bottom: 0;
  background: transparent;
}

.chat-session-window-picker :deep(.session-compact-trigger) {
  max-width: 360px;
}

.chat-session-window-picker :deep(.session-compact-title) {
  font-size: 14px;
}

.chat-session-window-picker :deep(.session-compact-dropdown) {
  left: 7px;
  top: calc(100% + 4px);
}

.chat-session-window-drag-region {
  -webkit-app-region: drag;
  min-width: 24px;
  flex: 1 1 40px;
  align-self: stretch;
}

.chat-session-window-controls {
  -webkit-app-region: no-drag;
  position: relative;
  z-index: 2;
  flex: 0 0 126px;
  min-width: 126px;
  height: 100%;
  display: flex;
  align-items: stretch;
  margin-left: auto;
}

.chat-session-window-control {
  width: 42px;
  height: 100%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.1s ease, color 0.1s ease;
}

.chat-session-window-control:hover,
.chat-session-window-control:focus-visible {
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.chat-session-window-control.is-close:hover,
.chat-session-window-control.is-close:focus-visible {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.chat-session-window-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 18px;
  background: var(--panel-bg);
  color: var(--text-secondary);
  font-size: 13px;
}

.chat-session-window-state.is-error {
  color: var(--status-danger-fg);
}

.chat-session-window-workspace {
  flex: 1;
  min-width: 0;
  min-height: 0;
}

:deep(.top-banner-host) {
  top: 44px;
}
</style>
