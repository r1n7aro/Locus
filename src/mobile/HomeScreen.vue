<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { formatSessionTime } from "../composables/useFormatters";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { isSessionUnread, reloadSessionAttention, sessionAttention } from "../services/sessionAttention";
import { watchWindowEventInterests } from "../services/windowEventInterests";
import type { WorkspaceRef } from "../services/project";
import type { SessionSummary, StreamEvent } from "../types";
import { t } from "../i18n";
import { liveCheckoutRefs, loadProjects, reloadProject, sessionWorkspaceRef, type ProjectEntry } from "./api";
import { onWorkspaceEvent } from "./events";

const props = defineProps<{
  desktopName: string;
  connected: boolean;
  /** Bumped after a reconnect or dropped events: reload everything. */
  refreshToken: number;
}>();

const emit = defineEmits<{
  openSession: [payload: { session: SessionSummary; workspaceRef: WorkspaceRef | null; projectName: string }];
  newSession: [payload: { workspaceRef: WorkspaceRef; projectName: string }];
  openFiles: [payload: { workspaceRef: WorkspaceRef; projectName: string }];
  openKnowledge: [payload: { workspaceRef: WorkspaceRef; projectName: string }];
  leave: [];
}>();

const SESSIONS_PER_PROJECT = 5;
const projects = ref<ProjectEntry[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const expanded = ref(new Set<string>());
const menuOpen = ref(false);

async function refresh() {
  if (loading.value) return;
  loading.value = true;
  error.value = null;
  // Results that arrived while this device was away.
  void reloadSessionAttention().catch(() => {});
  try {
    projects.value = await loadProjects();
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    loading.value = false;
  }
}

// Lifecycle events, titles and unread results of every open checkout
// (sessions may run in a worktree) keep the list current.
const stopInterests = watchWindowEventInterests(() => projects.value
  .flatMap((project) => liveCheckoutRefs(project))
  .map((workspaceRef) => ({ workspaceRef })));

const LIFECYCLE_TYPES = new Set(["runStart", "done", "error", "cancelled", "askUser", "toolConfirm", "inputAnswered"]);
const pendingReloads = new Set<string>();
let reloadTimer: ReturnType<typeof setTimeout> | null = null;

function scheduleReload(projectId: string) {
  pendingReloads.add(projectId);
  if (reloadTimer) return;
  reloadTimer = setTimeout(async () => {
    reloadTimer = null;
    const ids = [...pendingReloads];
    pendingReloads.clear();
    for (const id of ids) {
      const index = projects.value.findIndex((project) => project.projectId === id);
      if (index < 0) continue;
      const updated = await reloadProject(projects.value[index]!);
      const current = projects.value.findIndex((project) => project.projectId === id);
      if (current >= 0) projects.value.splice(current, 1, updated);
    }
  }, 400);
}

const stopEvents = onWorkspaceEvent((event) => {
  const relevant = event.eventName === "session-title-updated"
    || (event.eventName === "stream-event" && LIFECYCLE_TYPES.has((event.payload as StreamEvent).type));
  if (!relevant) return;
  const project = projects.value.find((candidate) => candidate.projectId === event.projectId
    || candidate.checkouts.some((checkout) => checkout.checkoutId === event.checkoutId));
  if (project) scheduleReload(project.projectId);
});

function onVisibilityChange() {
  if (document.visibilityState === "visible" && props.connected) void refresh();
}

onMounted(() => {
  void refresh();
  document.addEventListener("visibilitychange", onVisibilityChange);
});

onBeforeUnmount(() => {
  stopInterests();
  stopEvents();
  if (reloadTimer) clearTimeout(reloadTimer);
  document.removeEventListener("visibilitychange", onVisibilityChange);
});

watch(() => props.refreshToken, () => void refresh());
watch(() => props.connected, (connected, was) => {
  if (connected && !was) void refresh();
});

const { state: displaySettings } = useDisplaySettings();

function unread(session: SessionSummary): boolean {
  return displaySettings.showSessionUnreadIndicators && isSessionUnread(session.id);
}

function unreadCount(project: ProjectEntry): number {
  return project.sessions.reduce((count, session) => count + Number(unread(session)), 0);
}

/** Collapsed lists keep unread sessions beyond the first few. */
function visibleSessions(project: ProjectEntry): SessionSummary[] {
  return expanded.value.has(project.projectId)
    ? project.sessions
    : project.sessions.filter((session, index) => index < SESSIONS_PER_PROJECT || unread(session));
}

function toggleExpanded(projectId: string) {
  const next = new Set(expanded.value);
  if (next.has(projectId)) next.delete(projectId);
  else next.add(projectId);
  expanded.value = next;
}

function statusOf(session: SessionSummary): { label: string; tone: string } | null {
  switch (session.runtimeStatus) {
    case "running":
    case "starting":
    case "queued":
    case "finishing":
      return { label: t("mobile.home.statusRunning"), tone: "running" };
    case "waiting_input":
      return { label: t("mobile.home.statusWaiting"), tone: "waiting" };
    case "cancelling":
      return { label: t("mobile.home.statusStopping"), tone: "running" };
    case "error":
      return { label: t("mobile.home.statusError"), tone: "error" };
    default:
      break;
  }
  // A run that failed has no runtime status any more; its unread result does.
  const result = sessionAttention.value.sessions[session.id];
  return result?.kind === "error" && unread(session)
    ? { label: t("mobile.home.statusError"), tone: "error" }
    : null;
}

const isEmpty = computed(() => !loading.value && !error.value && projects.value.length === 0);

function open(project: ProjectEntry, session: SessionSummary) {
  emit("openSession", { session, workspaceRef: sessionWorkspaceRef(project, session), projectName: project.name });
}
</script>

<template>
  <section class="m-screen m-home">
    <header class="m-topbar">
      <div class="m-topbar-text">
        <span class="m-topbar-title">Locus</span>
        <span class="m-topbar-sub">
          <span class="m-dot" :class="{ 'is-online': connected }" />
          {{ desktopName || t("mobile.home.desktop") }}
        </span>
      </div>
      <button
        class="m-icon-button"
        type="button"
        :aria-label="t('mobile.common.refresh')"
        :disabled="loading || !connected"
        @click="refresh"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true" :class="{ 'is-spinning': loading }">
          <path d="M20 12a8 8 0 1 1-2.34-5.66M20 4v5h-5" />
        </svg>
      </button>
      <div class="m-menu-anchor">
        <button
          class="m-icon-button"
          type="button"
          :aria-label="t('mobile.common.more')"
          :aria-expanded="menuOpen"
          @click="menuOpen = !menuOpen"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5.5h.01M12 12h.01M12 18.5h.01" /></svg>
        </button>
        <div v-if="menuOpen" class="m-menu" role="menu" @click="menuOpen = false">
          <button class="m-menu-item" type="button" role="menuitem" @click="emit('leave')">
            {{ t("mobile.home.switchDesktop") }}
          </button>
        </div>
      </div>
    </header>

    <div class="m-scroll">
      <div v-if="error" class="m-notice" role="alert">{{ error }}</div>
      <div v-if="loading && projects.length === 0" class="m-placeholder">{{ t("mobile.common.loading") }}</div>
      <div v-if="isEmpty" class="m-placeholder">{{ t("mobile.home.empty") }}</div>

      <article v-for="project in projects" :key="project.projectId" class="m-project">
        <header class="m-project-header">
          <h2 class="m-project-name">{{ project.name }}</h2>
          <span
            v-if="unreadCount(project) > 0"
            class="m-unread-count"
            :aria-label="t('mobile.home.unreadCount', String(unreadCount(project)))"
          >{{ unreadCount(project) }}</span>
          <button
            class="m-pill-button"
            type="button"
            :disabled="!project.workspaceRef || !connected"
            :title="project.workspaceRef ? '' : t('mobile.home.openOnDesktop')"
            @click="project.workspaceRef && emit('newSession', { workspaceRef: project.workspaceRef, projectName: project.name })"
          >
            {{ t("mobile.home.newSession") }}
          </button>
        </header>
        <p v-if="project.error" class="m-project-error">{{ project.error }}</p>
        <p v-else-if="project.sessions.length === 0" class="m-project-empty">{{ t("mobile.home.noSessions") }}</p>
        <ul v-else class="m-session-list">
          <li v-for="session in visibleSessions(project)" :key="session.id">
            <button
              class="m-session-row"
              :class="{ 'is-unread': unread(session) }"
              type="button"
              @click="open(project, session)"
            >
              <span class="m-session-title">
                <span
                  v-if="unread(session)"
                  class="m-unread-dot"
                  role="img"
                  :aria-label="t('chat.session.unread')"
                />
                <span class="m-session-title-text">{{ session.title || t("mobile.session.untitled") }}</span>
              </span>
              <span class="m-session-meta">
                <span
                  v-if="statusOf(session)"
                  class="m-status"
                  :data-tone="statusOf(session)!.tone"
                >{{ statusOf(session)!.label }}</span>
                <span class="m-session-time">{{ formatSessionTime(session.updatedAt) }}</span>
              </span>
            </button>
          </li>
        </ul>
        <button
          v-if="project.sessions.length > SESSIONS_PER_PROJECT"
          class="m-text-button m-more"
          type="button"
          @click="toggleExpanded(project.projectId)"
        >
          {{ expanded.has(project.projectId)
            ? t("mobile.home.showLess")
            : t("mobile.home.showAll", String(project.sessions.length)) }}
        </button>
        <p v-if="!project.workspaceRef" class="m-project-hint">{{ t("mobile.home.openOnDesktop") }}</p>
        <div v-else class="m-project-actions">
          <button
            class="m-project-action"
            type="button"
            @click="emit('openFiles', { workspaceRef: project.workspaceRef, projectName: project.name })"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" /></svg>
            {{ t("mobile.files.title") }}
          </button>
          <button
            class="m-project-action"
            type="button"
            @click="emit('openKnowledge', { workspaceRef: project.workspaceRef, projectName: project.name })"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 4.5h9.5a3 3 0 0 1 3 3v12h-10a2.5 2.5 0 0 1-2.5-2.5zM5 17a2.5 2.5 0 0 1 2.5-2.5h10M9 8.5h5" /></svg>
            {{ t("mobile.knowledge.title") }}
          </button>
        </div>
      </article>
    </div>
  </section>
</template>
