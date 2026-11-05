<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import type { WorkspaceRef } from "../services/project";
import type { RemoteReference } from "../services/remoteClient";
import {
  onSessionAttentionResult,
  reloadSessionAttention,
  type SessionAttentionChange,
  type SessionAttentionKind,
} from "../services/sessionAttention";
import { useNotificationStore } from "../stores/notification";
import type { KnowledgeDocumentType, SessionSummary } from "../types";
import { t } from "../i18n";
import type { AppBridge, ConnectionStatus } from "./bridge";
import DocumentScreen from "./DocumentScreen.vue";
import FilesScreen from "./FilesScreen.vue";
import HomeScreen from "./HomeScreen.vue";
import ImageScreen from "./ImageScreen.vue";
import KnowledgeScreen from "./KnowledgeScreen.vue";
import SessionScreen from "./SessionScreen.vue";
import { checkoutRoot, findListedSession, listedSessionsRevision } from "./api";
import { onWorkspaceEvent } from "./events";
import { loadModelCatalog } from "./models";
import { relativeToRoot } from "./references";
import { baseName, type MobileRoute } from "./routes";

const props = defineProps<{ bridge: AppBridge }>();

const status = ref<ConnectionStatus>({ state: "connecting", desktopName: "", deviceName: "" });
const fullControl = ref(false);
const desktopName = ref("");
/** Screens above the project list; every entry stays mounted (scroll
 * positions, running sessions) and only the top one is shown. */
const stack = shallowRef<MobileRoute[]>([]);
/** Bumped when the desktop dropped events or the link was re-established;
 * the visible screen is rebuilt from fresh snapshots. */
const resyncCount = ref(0);
let routeSeq = 0;

const connected = computed(() => status.value.state === "connected");
const top = computed(() => stack.value[stack.value.length - 1] ?? null);
const banner = computed(() => {
  switch (status.value.state) {
    case "connecting":
      return { tone: "info", text: t("mobile.connection.connecting") };
    case "reconnecting":
      return { tone: "warn", text: t("mobile.connection.reconnecting") };
    case "revoked":
      return { tone: "danger", text: t("mobile.connection.revoked") };
    case "closed":
      return { tone: "danger", text: t("mobile.connection.closed") };
    default:
      return null;
  }
});

async function refreshDeviceInfo() {
  try {
    const info = await props.bridge.deviceInfo();
    fullControl.value = info.fullControl;
    desktopName.value = info.desktopName;
  } catch {
    // Keeps the previous values; the banner already reports connection trouble.
  }
}

function applyStatus(next: ConnectionStatus) {
  const wasConnected = connected.value;
  status.value = next;
  if (next.desktopName) desktopName.value = next.desktopName;
  if (next.state === "connected" && !wasConnected) {
    void refreshDeviceInfo();
    refreshModelCatalog();
    refreshAttention();
  }
}

/** Results that arrived while the link was down. */
function refreshAttention() {
  void reloadSessionAttention().catch(() => {});
}

/** Model and provider settings may have changed while the link was down. */
function refreshModelCatalog() {
  void loadModelCatalog({ refresh: true }).catch(() => {});
}

// ----- navigation: one history entry per screen, so the Android back gesture
// pops it.
type NewRoute = MobileRoute extends infer R ? (R extends MobileRoute ? Omit<R, "key"> : never) : never;

function push(route: NewRoute) {
  routeSeq += 1;
  stack.value = [...stack.value, { ...route, key: `${route.kind}:${Date.now()}:${routeSeq}` } as MobileRoute];
  history.pushState({ locusDepth: stack.value.length }, "");
}

function back() {
  if (history.state?.locusDepth) history.back();
  else stack.value = stack.value.slice(0, -1);
}

function onPopState() {
  const depth = typeof history.state?.locusDepth === "number" ? history.state.locusDepth : 0;
  if (stack.value.length > depth) stack.value = stack.value.slice(0, depth);
}

/** The workspace of the screen a reference was opened from. */
function currentWorkspaceRef(): WorkspaceRef | null {
  for (let index = stack.value.length - 1; index >= 0; index -= 1) {
    const route = stack.value[index]!;
    if (route.kind === "image") continue;
    if (route.kind === "document") return route.source.workspaceRef;
    if (route.workspaceRef) return route.workspaceRef;
  }
  return null;
}

/** Name shown for a workspace that was reached through a reference. */
function workspaceName(workspaceRef: WorkspaceRef): string {
  for (const route of stack.value) {
    if ((route.kind === "files" || route.kind === "knowledge") && route.workspaceRef.checkoutId === workspaceRef.checkoutId) {
      return route.projectName;
    }
  }
  const root = checkoutRoot(workspaceRef.checkoutId);
  return root ? baseName(root) : "";
}

function openReference(reference: RemoteReference) {
  const workspaceRef = reference.workspaceRef ?? currentWorkspaceRef();
  if (!workspaceRef) return;
  if (reference.kind === "folder") {
    // The file list browses workspace-relative folders.
    const path = relativeToRoot(reference.path, checkoutRoot(workspaceRef.checkoutId));
    if (path === null) return;
    push({ kind: "files", workspaceRef, projectName: workspaceName(workspaceRef), path });
  } else if (reference.kind === "knowledge") {
    push({ kind: "document", title: baseName(reference.path), source: { kind: "knowledge", workspaceRef, docType: reference.docType, path: reference.path } });
  } else {
    push({ kind: "document", title: baseName(reference.path), source: { kind: "file", workspaceRef, path: reference.path } });
  }
}

function openImage(src: string, title: string) {
  if (src) push({ kind: "image", src, title });
}

function onOpenSession(payload: { session: SessionSummary; workspaceRef: WorkspaceRef | null }) {
  openSession(payload.session.id, payload.session.title, payload.workspaceRef);
}

/** Shows a session, going back to it when it is already on the stack. */
function openSession(sessionId: string, title: string, workspaceRef: WorkspaceRef | null) {
  const index = stack.value.findIndex((route) => route.kind === "session" && route.sessionId === sessionId);
  if (index < 0) {
    push({ kind: "session", sessionId, workspaceRef, title });
    return;
  }
  const steps = stack.value.length - 1 - index;
  if (steps > 0) history.go(-steps);
}

/** `/clear` in a session: a new conversation takes its place. */
function replaceWithNewSession(route: MobileRoute) {
  if (route.kind !== "session" || stack.value[stack.value.length - 1]?.key !== route.key) return;
  routeSeq += 1;
  const next: MobileRoute = {
    kind: "session",
    key: `session:${Date.now()}:${routeSeq}`,
    sessionId: null,
    workspaceRef: route.workspaceRef,
    title: t("mobile.session.newTitle"),
  };
  stack.value = [...stack.value.slice(0, -1), next];
}

// ----- titles follow renames and generated titles: live events, and the
// session list for changes made while this device was away.
function applyTitle(sessionId: string, title: string) {
  if (!title.trim()) return;
  if (!stack.value.some((route) => route.kind === "session" && route.sessionId === sessionId && route.title !== title)) return;
  stack.value = stack.value.map((route) => (
    route.kind === "session" && route.sessionId === sessionId ? { ...route, title } : route
  ));
}

watch(listedSessionsRevision, () => {
  for (const route of stack.value) {
    if (route.kind !== "session" || !route.sessionId) continue;
    const title = findListedSession(route.sessionId)?.session.title;
    if (title) applyTitle(route.sessionId, title);
  }
});

function onNewSession(payload: { workspaceRef: WorkspaceRef }) {
  push({ kind: "session", sessionId: null, workspaceRef: payload.workspaceRef, title: t("mobile.session.newTitle") });
}

function onOpenFiles(payload: { workspaceRef: WorkspaceRef; projectName: string }) {
  push({ kind: "files", workspaceRef: payload.workspaceRef, projectName: payload.projectName, path: "" });
}

function onOpenKnowledge(payload: { workspaceRef: WorkspaceRef; projectName: string }) {
  push({ kind: "knowledge", workspaceRef: payload.workspaceRef, projectName: payload.projectName, docType: null, path: "" });
}

// ----- foreground reminders: a result in another session while the app is
// open (there is no background push).
interface Reminder {
  sessionId: string;
  kind: SessionAttentionKind;
  title: string;
  workspaceRef: WorkspaceRef | null;
}

const REMINDER_MS = 6000;
const reminder = ref<Reminder | null>(null);
let reminderTimer: ReturnType<typeof setTimeout> | null = null;

const REMINDER_LABELS: Record<SessionAttentionKind, string> = {
  done: "notifications.chatDoneTitle",
  error: "notifications.chatErrorTitle",
  askUser: "notifications.askUserTitle",
  toolConfirm: "notifications.toolConfirmTitle",
  knowledgeProposal: "notifications.knowledgeProposalTitle",
};

function reminderTone(kind: SessionAttentionKind): string {
  if (kind === "error") return "error";
  return kind === "done" ? "done" : "waiting";
}

function dismissReminder() {
  reminder.value = null;
  if (reminderTimer) clearTimeout(reminderTimer);
  reminderTimer = null;
}

/** Waits this long for the session list to include a session started
 * elsewhere (it reloads on the session's first events). */
const REMINDER_LIST_WAIT_MS = 3000;

function onAttentionResult(change: SessionAttentionChange) {
  if (showReminder(change)) return;
  const stop = watch(listedSessionsRevision, () => {
    if (showReminder(change)) stop();
  });
  setTimeout(stop, REMINDER_LIST_WAIT_MS);
}

/** False while the session is not known yet. */
function showReminder(change: SessionAttentionChange): boolean {
  if (document.visibilityState !== "visible") return true;
  const current = top.value;
  if (current?.kind === "session" && current.sessionId === change.sessionId) return true;
  const open = stack.value.find((route) => route.kind === "session" && route.sessionId === change.sessionId);
  const listed = findListedSession(change.sessionId);
  // Sub-agent sessions are never listed; their parent reports the outcome.
  if (!open && !listed) return false;
  reminder.value = {
    sessionId: change.sessionId,
    kind: change.entry.kind,
    title: (open?.kind === "session" ? open.title : "") || listed?.session.title || t("mobile.session.untitled"),
    workspaceRef: open?.kind === "session" ? open.workspaceRef : listed?.workspaceRef ?? null,
  };
  if (reminderTimer) clearTimeout(reminderTimer);
  reminderTimer = setTimeout(dismissReminder, REMINDER_MS);
  return true;
}

function openReminder() {
  const current = reminder.value;
  dismissReminder();
  if (current) openSession(current.sessionId, current.title, current.workspaceRef);
}

// Notices of shared components (image limits, failed actions).
const notificationStore = useNotificationStore();

function onSessionCreated(route: MobileRoute, sessionId: string) {
  // Keep the route key: the running session state lives under it.
  if (route.kind !== "session") return;
  stack.value = stack.value.map((entry) => (entry.key === route.key ? { ...route, sessionId } : entry));
}

const releases: Array<() => void> = [];
onMounted(async () => {
  window.addEventListener("popstate", onPopState);
  props.bridge.setReferenceOpener(openReference);
  releases.push(() => props.bridge.setReferenceOpener(null));
  releases.push(props.bridge.onStatus(applyStatus));
  releases.push(props.bridge.onResync(() => {
    resyncCount.value += 1;
    refreshModelCatalog();
    refreshAttention();
  }));
  releases.push(onSessionAttentionResult(onAttentionResult));
  releases.push(onWorkspaceEvent((event) => {
    if (event.eventName !== "session-title-updated") return;
    const payload = event.payload as { sessionId?: string; title?: string };
    if (payload.sessionId && payload.title) applyTitle(payload.sessionId, payload.title);
  }));
  try {
    applyStatus(await props.bridge.status());
  } catch {
    applyStatus({ state: "reconnecting", desktopName: "", deviceName: "" });
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("popstate", onPopState);
  releases.forEach((release) => release());
  dismissReminder();
});
</script>

<template>
  <div class="m-app" :data-connection="status.state">
    <div v-if="banner" class="m-banner" :data-tone="banner.tone" role="status">
      <span>{{ banner.text }}</span>
      <button
        v-if="status.state === 'revoked' || status.state === 'closed'"
        class="m-text-button"
        type="button"
        @click="bridge.leave()"
      >
        {{ t("mobile.connection.backToDesktops") }}
      </button>
    </div>
    <template v-for="route in stack" :key="route.kind === 'session' ? `${route.key}:${resyncCount}` : route.key">
      <SessionScreen
        v-if="route.kind === 'session'"
        v-show="route === top"
        :session-key="route.key"
        :session-id="route.sessionId"
        :workspace-ref="route.workspaceRef"
        :title="route.title"
        :can-approve-tools="fullControl"
        :connected="connected"
        :active="route === top"
        @back="back"
        @session-created="onSessionCreated(route, $event)"
        @open-image="openImage($event, route.title)"
        @open-session="openSession($event.sessionId, $event.title, route.workspaceRef)"
        @new-session="replaceWithNewSession(route)"
      />
      <FilesScreen
        v-else-if="route.kind === 'files'"
        v-show="route === top"
        :workspace-ref="route.workspaceRef"
        :project-name="route.projectName"
        :path="route.path"
        :connected="connected"
        :active="route === top"
        :refresh-token="resyncCount"
        @back="back"
        @open-directory="push({ kind: 'files', workspaceRef: route.workspaceRef, projectName: route.projectName, path: $event })"
        @open-file="(path: string, name: string) => push({ kind: 'document', title: name, source: { kind: 'file', workspaceRef: route.workspaceRef, path } })"
      />
      <KnowledgeScreen
        v-else-if="route.kind === 'knowledge'"
        v-show="route === top"
        :workspace-ref="route.workspaceRef"
        :project-name="route.projectName"
        :doc-type="route.docType"
        :path="route.path"
        :connected="connected"
        :active="route === top"
        :refresh-token="resyncCount"
        @back="back"
        @open-folder="(docType: KnowledgeDocumentType, path: string) => push({ kind: 'knowledge', workspaceRef: route.workspaceRef, projectName: route.projectName, docType, path })"
        @open-document="(docType: KnowledgeDocumentType, path: string, title: string) => push({ kind: 'document', title: title || baseName(path), source: { kind: 'knowledge', workspaceRef: route.workspaceRef, docType, path } })"
      />
      <DocumentScreen
        v-else-if="route.kind === 'document'"
        v-show="route === top"
        :source="route.source"
        :title="route.title"
        :connected="connected"
        :active="route === top"
        :refresh-token="resyncCount"
        @back="back"
        @open-image="openImage"
      />
      <ImageScreen
        v-else-if="route.kind === 'image'"
        v-show="route === top"
        :src="route.src"
        :title="route.title"
        @back="back"
      />
    </template>
    <HomeScreen
      v-show="!top"
      :desktop-name="desktopName"
      :connected="connected"
      :refresh-token="resyncCount"
      @open-session="onOpenSession"
      @new-session="onNewSession"
      @open-files="onOpenFiles"
      @open-knowledge="onOpenKnowledge"
      @leave="bridge.leave()"
    />
    <div class="m-alerts" aria-live="polite">
      <div v-if="reminder" class="m-reminder" :data-tone="reminderTone(reminder.kind)" role="status">
        <button class="m-reminder-open" type="button" @click="openReminder">
          <span class="m-reminder-kind">{{ t(REMINDER_LABELS[reminder.kind]) }}</span>
          <span class="m-reminder-title">{{ reminder.title }}</span>
        </button>
        <button class="m-icon-button" type="button" :aria-label="t('common.close')" @click="dismissReminder">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7l10 10M17 7L7 17" /></svg>
        </button>
      </div>
      <div
        v-for="notice in notificationStore.visibleNotices"
        :key="notice.id"
        class="m-alert"
        :data-level="notice.level"
        role="status"
      >
        <span class="m-alert-text">{{ notice.message }}</span>
        <button
          class="m-icon-button"
          type="button"
          :aria-label="t('common.close')"
          @click="notificationStore.removeNotice(notice.id)"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7l10 10M17 7L7 17" /></svg>
        </button>
      </div>
    </div>
  </div>
</template>
