<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { ArchiveRestore, RefreshCw } from "lucide";
import { t } from "../../i18n";
import { formatSessionTime } from "../../composables/useFormatters";
import { normalizeAppError } from "../../services/errors";
import type { WorkspaceRef } from "../../services/project";
import {
  listArchivedCheckoutSessions,
  loadSession,
  unarchiveSession,
} from "../../services/session";
import { useChatStore } from "../../stores/chat";
import { useNotificationStore } from "../../stores/notification";
import { useWorkspaceExplorerStore } from "../../stores/workspaceExplorer";
import type { SessionDetail, SessionSummary } from "../../types";
import ChatTranscript from "../chat/ChatTranscript.vue";
import ThinkingPanel from "../ThinkingPanel.vue";
import BaseButton from "../ui/BaseButton.vue";
import LucideIcon from "../icons/LucideIcon.vue";

const props = withDefaults(defineProps<{
  projectId: string;
  workspaceRef: WorkspaceRef | null;
  active?: boolean;
}>(), {
  active: true,
});

const chatStore = useChatStore();
const notificationStore = useNotificationStore();
const explorerStore = useWorkspaceExplorerStore();

const archivedSessions = ref<SessionSummary[]>([]);
const selectedSessionId = ref<string | null>(null);
const selectedDetail = ref<SessionDetail | null>(null);
const listLoading = ref(false);
const detailLoading = ref(false);
const loadFailed = ref(false);
const unarchivingIds = ref<Set<string>>(new Set());
const thinkingText = ref("");
const lightboxSrc = ref("");

let listRequestId = 0;
let detailRequestId = 0;

const workspaceScopeKey = computed(() => {
  const workspaceRef = props.workspaceRef;
  return workspaceRef
    ? `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? ""}`
    : "";
});

const selectedSummary = computed(() => (
  archivedSessions.value.find((session) => session.id === selectedSessionId.value) ?? null
));

const activeSessionSignature = computed(() => (
  (explorerStore.resources[props.projectId]?.sessions ?? [])
    .map((session) => `${session.id}:${session.updatedAt}`)
    .join("|")
));

function isUnarchiving(sessionId: string): boolean {
  return unarchivingIds.value.has(sessionId);
}

function resetSelection(): void {
  detailRequestId += 1;
  archivedSessions.value = [];
  selectedSessionId.value = null;
  selectedDetail.value = null;
  loadFailed.value = false;
  thinkingText.value = "";
}

async function loadArchivedDetail(sessionId: string | null): Promise<void> {
  thinkingText.value = "";
  if (!sessionId) {
    selectedDetail.value = null;
    detailLoading.value = false;
    return;
  }

  const requestId = ++detailRequestId;
  detailLoading.value = true;
  try {
    const detail = await loadSession(sessionId);
    if (requestId !== detailRequestId || selectedSessionId.value !== sessionId) return;
    selectedDetail.value = detail;
  } catch (error) {
    if (requestId !== detailRequestId) return;
    selectedDetail.value = null;
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", t("development.archived.loadFailed", normalized.message), {
      code: normalized.code,
      operation: "loadArchivedSessionDetail",
    });
  } finally {
    if (requestId === detailRequestId) detailLoading.value = false;
  }
}

async function refreshArchived(options: { preserveSelection?: boolean } = {}): Promise<void> {
  const workspaceRef = props.workspaceRef;
  if (!workspaceRef) {
    resetSelection();
    return;
  }

  const requestScopeKey = workspaceScopeKey.value;
  const requestId = ++listRequestId;
  listLoading.value = true;
  try {
    const sessions = await listArchivedCheckoutSessions(workspaceRef);
    if (requestId !== listRequestId || requestScopeKey !== workspaceScopeKey.value) return;
    archivedSessions.value = sessions;
    loadFailed.value = false;

    const currentId = options.preserveSelection !== false
      && selectedSessionId.value
      && sessions.some((session) => session.id === selectedSessionId.value)
      ? selectedSessionId.value
      : null;
    const nextId = currentId ?? sessions[0]?.id ?? null;
    selectedSessionId.value = nextId;
    await loadArchivedDetail(nextId);
  } catch (error) {
    if (requestId !== listRequestId) return;
    loadFailed.value = true;
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", t("development.archived.loadFailed", normalized.message), {
      code: normalized.code,
      operation: "loadArchivedSessions",
    });
  } finally {
    if (requestId === listRequestId) listLoading.value = false;
  }
}

function selectArchivedSession(sessionId: string): void {
  if (selectedSessionId.value === sessionId) return;
  selectedSessionId.value = sessionId;
  void loadArchivedDetail(sessionId);
}

async function handleUnarchive(sessionId: string): Promise<void> {
  if (!sessionId || isUnarchiving(sessionId)) return;
  unarchivingIds.value = new Set(unarchivingIds.value).add(sessionId);
  try {
    await unarchiveSession(sessionId);
    await Promise.all([
      chatStore.refreshSessions(),
      explorerStore.refreshProjectSessions(props.projectId),
    ]);
    await refreshArchived();
    notificationStore.addNotice("success", t("chat.session.unarchived"), {
      operation: "unarchiveSession",
    });
  } catch (error) {
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", t("development.archived.unarchiveFailed", normalized.message), {
      code: normalized.code,
      operation: "unarchiveSession",
    });
  } finally {
    const next = new Set(unarchivingIds.value);
    next.delete(sessionId);
    unarchivingIds.value = next;
  }
}

function handleTranscriptClick(event: MouseEvent): void {
  const target = event.target instanceof Element ? event.target : null;
  const image = target?.closest("img") as HTMLImageElement | null;
  if (!image?.src) return;
  event.preventDefault();
  lightboxSrc.value = image.src;
}

watch(
  workspaceScopeKey,
  () => {
    resetSelection();
    if (props.active && workspaceScopeKey.value) {
      void refreshArchived({ preserveSelection: false });
    }
  },
  { immediate: true },
);

watch(
  () => props.active,
  (active, previous) => {
    if (active && !previous && workspaceScopeKey.value) void refreshArchived();
  },
);

watch(activeSessionSignature, (signature, previous) => {
  if (props.active && previous !== undefined && signature !== previous && workspaceScopeKey.value) {
    void refreshArchived();
  }
});
</script>

<template>
  <div class="archived-workbench">
    <aside class="archived-sidebar">
      <div class="archived-sidebar-toolbar">
        <span class="archived-sidebar-title">{{ t("app.tab.archived") }}</span>
        <span v-if="archivedSessions.length" class="archived-count">{{ archivedSessions.length }}</span>
        <button
          type="button"
          class="archived-icon-button"
          :disabled="listLoading"
          :title="t('common.refresh')"
          :aria-label="t('common.refresh')"
          @click="refreshArchived()"
        >
          <LucideIcon :icon="RefreshCw" :size="13" :stroke-width="2" />
        </button>
      </div>

      <div v-if="listLoading && archivedSessions.length === 0" class="archived-sidebar-state">
        {{ t("common.loading") }}
      </div>
      <div v-else-if="loadFailed && archivedSessions.length === 0" class="archived-sidebar-state">
        <span>{{ t("development.archived.loadFailedShort") }}</span>
        <BaseButton size="sm" @click="refreshArchived({ preserveSelection: false })">
          {{ t("common.refresh") }}
        </BaseButton>
      </div>
      <div v-else-if="archivedSessions.length === 0" class="archived-sidebar-state">
        {{ t("development.archived.empty") }}
      </div>
      <div v-else class="archived-session-list">
        <div
          v-for="session in archivedSessions"
          :key="session.id"
          class="archived-session-row"
          :class="{ active: session.id === selectedSessionId }"
        >
          <button
            type="button"
            class="archived-session-select"
            @click="selectArchivedSession(session.id)"
          >
            <span class="archived-session-title">
              {{ session.title || t("chat.session.newSession") }}
            </span>
            <span class="archived-session-time">{{ formatSessionTime(session.updatedAt) }}</span>
          </button>
          <button
            type="button"
            class="archived-session-action"
            :disabled="isUnarchiving(session.id)"
            :title="t('chat.session.unarchive')"
            :aria-label="t('chat.session.unarchive')"
            @click="handleUnarchive(session.id)"
          >
            <LucideIcon :icon="ArchiveRestore" :size="13" :stroke-width="2" />
          </button>
        </div>
      </div>
    </aside>

    <section class="archived-conversation">
      <div v-if="selectedSummary" class="archived-conversation-toolbar">
        <div class="archived-conversation-heading">
          <span class="archived-conversation-title">
            {{ selectedSummary.title || t("chat.session.newSession") }}
          </span>
          <span class="archived-conversation-time">
            {{ formatSessionTime(selectedSummary.updatedAt) }}
          </span>
        </div>
        <BaseButton
          size="sm"
          :disabled="isUnarchiving(selectedSummary.id)"
          @click="handleUnarchive(selectedSummary.id)"
        >
          <LucideIcon :icon="ArchiveRestore" :size="13" :stroke-width="2" />
          {{ t("chat.session.unarchive") }}
        </BaseButton>
      </div>

      <div v-if="detailLoading" class="archived-conversation-state">
        {{ t("common.loading") }}
      </div>
      <div v-else-if="!selectedDetail" class="archived-conversation-state">
        {{ t("development.archived.selectSession") }}
      </div>
      <div v-else class="archived-conversation-body">
        <ChatTranscript
          variant="session"
          :session-key="`archived:${selectedDetail.id}`"
          :workspace-ref="workspaceRef"
          :messages="selectedDetail.messages"
          streaming-text=""
          :is-streaming="false"
          :is-thinking="false"
          :active-tool-calls="[]"
          user-label="You"
          assistant-label="Locus"
          :handoff-label="t('chat.transcript.handoff')"
          :waiting-label="t('chat.transcript.waiting')"
          :compacting-label="t('chat.transcript.compacting')"
          :compacted-label="t('chat.transcript.compacted')"
          :thinking-active-label="t('chat.transcript.thinking')"
          :thought-duration-label="t('chat.transcript.thoughtDuration', '{0}')"
          :thought-moment-label="t('chat.transcript.thoughtMoment')"
          :empty-title="t('development.archived.emptyConversation')"
          enable-intent-badges
          show-user-images
          user-content-mode="asset"
          @content-click="handleTranscriptClick"
          @open-image="lightboxSrc = $event"
          @open-thinking="thinkingText = $event"
        />
        <ThinkingPanel
          v-if="thinkingText"
          :text="thinkingText"
          :is-thinking="false"
          @close="thinkingText = ''"
        />
      </div>
    </section>

    <Transition name="archived-lightbox">
      <div v-if="lightboxSrc" class="archived-lightbox" @click="lightboxSrc = ''">
        <img :src="lightboxSrc" :alt="t('development.archived.imagePreview')" @click.stop />
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.archived-workbench {
  display: grid;
  grid-template-columns: minmax(220px, 280px) minmax(0, 1fr);
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: var(--panel-bg);
}

.archived-sidebar,
.archived-conversation {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
}

.archived-sidebar {
  background: var(--sidebar-bg);
  border-right: 1px solid var(--border-color);
}

.archived-sidebar-toolbar,
.archived-conversation-toolbar {
  display: flex;
  align-items: center;
  min-height: 38px;
  padding: 0 10px;
  border-bottom: 1px solid var(--border-color);
  flex: 0 0 auto;
}

.archived-sidebar-title,
.archived-conversation-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-size: 13px;
  font-weight: 600;
}

.archived-count,
.archived-session-time,
.archived-conversation-time {
  color: var(--text-secondary);
  font-size: 11px;
}

.archived-count {
  margin-left: 6px;
}

.archived-icon-button,
.archived-session-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.archived-icon-button {
  width: 26px;
  height: 26px;
  margin-left: auto;
}

.archived-icon-button:hover:not(:disabled),
.archived-session-action:hover:not(:disabled),
.archived-icon-button:focus-visible,
.archived-session-action:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.archived-icon-button:disabled,
.archived-session-action:disabled {
  opacity: 0.45;
  cursor: default;
}

.archived-session-list {
  flex: 1 1 0;
  min-height: 0;
  overflow-y: auto;
  padding: 4px;
}

.archived-session-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 26px;
  align-items: center;
  min-height: 34px;
  border-radius: 4px;
}

.archived-session-row:hover,
.archived-session-row.active {
  background: var(--hover-bg);
}

.archived-session-row.active {
  box-shadow: inset 2px 0 0 var(--accent-color);
}

.archived-session-select {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
  min-height: 34px;
  padding: 0 4px 0 9px;
  border: 0;
  background: transparent;
  color: var(--text-color);
  text-align: left;
  cursor: pointer;
}

.archived-session-select:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: -1px;
}

.archived-session-title {
  flex: 1 1 0;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
}

.archived-session-time {
  flex: 0 0 auto;
}

.archived-session-action {
  width: 24px;
  height: 24px;
  opacity: 0;
}

.archived-session-row:hover .archived-session-action,
.archived-session-row.active .archived-session-action,
.archived-session-action:focus-visible {
  opacity: 1;
}

.archived-sidebar-state,
.archived-conversation-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  flex: 1 1 0;
  min-height: 0;
  padding: 20px;
  color: var(--text-secondary);
  font-size: 13px;
  text-align: center;
}

.archived-conversation-toolbar {
  justify-content: space-between;
  gap: 12px;
  background: var(--panel-bg);
}

.archived-conversation-heading {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.archived-conversation-body {
  display: flex;
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.archived-conversation-body :deep(.chat-transcript-scroll.is-session) {
  background: var(--panel-bg);
}

.archived-lightbox {
  position: fixed;
  inset: 0;
  z-index: 9999;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.7);
  backdrop-filter: blur(4px);
  cursor: zoom-out;
}

.archived-lightbox img {
  max-width: 90vw;
  max-height: 90vh;
  border-radius: 8px;
  object-fit: contain;
  cursor: default;
}

.archived-lightbox-enter-active,
.archived-lightbox-leave-active {
  transition: opacity 150ms ease;
}

.archived-lightbox-enter-from,
.archived-lightbox-leave-to {
  opacity: 0;
}

@media (max-width: 720px) {
  .archived-workbench {
    grid-template-columns: minmax(180px, 36%) minmax(0, 1fr);
  }

  .archived-session-time,
  .archived-conversation-time {
    display: none;
  }
}
</style>
