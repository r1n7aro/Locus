import { Window } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  requestPermission,
} from "@tauri-apps/plugin-notification";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { t } from "../i18n";
import { sendSystemNotification } from "./system";
import { getUnityEmbedFocusDebugSnapshot } from "./unity";
import type { StreamEvent, ToolConfirmDisplay } from "../types";

interface StreamNotificationContext {
  sessionTitle?: string | null;
}

type NotifiableStreamEvent = Extract<
  StreamEvent,
  { type: "done" | "askUser" | "error" | "toolConfirm" | "knowledgeProposal" }
>;

const MAX_RECENT_NOTIFICATION_KEYS = 128;

const recentNotificationKeys = new Map<string, number>();

let permissionRequested = false;
let permissionCheckInFlight: Promise<boolean> | null = null;

function isNotifiableStreamEvent(event: StreamEvent): event is NotifiableStreamEvent {
  return (
    event.type === "done"
    || event.type === "askUser"
    || event.type === "error"
    || event.type === "toolConfirm"
    || event.type === "knowledgeProposal"
  );
}

function rememberNotificationKey(key: string) {
  recentNotificationKeys.set(key, Date.now());
  while (recentNotificationKeys.size > MAX_RECENT_NOTIFICATION_KEYS) {
    const oldestKey = recentNotificationKeys.keys().next().value;
    if (!oldestKey) break;
    recentNotificationKeys.delete(oldestKey);
  }
}

function getNotificationKey(event: NotifiableStreamEvent): string {
  switch (event.type) {
    case "done":
    case "error":
      return `${event.runId}:${event.type}`;
    case "askUser":
      return `ask:${event.questionId}`;
    case "toolConfirm":
      return `confirm:${event.questionId}`;
    case "knowledgeProposal": {
      const proposalId = event.message.knowledgeProposal?.proposalId?.trim();
      return proposalId ? `proposal:${proposalId}` : `proposal-message:${event.message.id}`;
    }
  }
}

function summarizeText(text: string | null | undefined, maxLength = 140): string {
  const normalized = text?.replace(/\s+/g, " ").trim() ?? "";
  if (!normalized) return "";
  if (normalized.length <= maxLength) return normalized;
  return `${normalized.slice(0, maxLength - 3).trimEnd()}...`;
}

function summarizeUnityStatusChange(requestedStatus: string): string {
  const key = `chat.toolConfirm.unityStatus.title.${requestedStatus}`;
  const title = t(key);
  return title === key ? t("chat.toolConfirm.unityStatus.title") : title;
}

function summarizeToolConfirmDisplay(display: ToolConfirmDisplay): string {
  if (display.kind === "basic") {
    return summarizeText(display.toolName);
  }
  if (display.kind === "unityEditorStatusChange") {
    return summarizeUnityStatusChange(display.requestedStatus);
  }
  return summarizeText(display.path);
}

function summarizeKnowledgeProposal(event: Extract<StreamEvent, { type: "knowledgeProposal" }>): string {
  const proposal = event.message.knowledgeProposal;
  if (!proposal || proposal.status !== "pending") {
    return t("notifications.knowledgeProposalFallback");
  }

  const firstTarget = summarizeText(proposal.items[0]?.target);
  if (proposal.items.length === 1 && firstTarget) {
    return t("notifications.knowledgeProposalSingle", firstTarget);
  }
  if (proposal.items.length > 1) {
    return t("notifications.knowledgeProposalMultiple", proposal.items.length);
  }
  return t("notifications.knowledgeProposalFallback");
}

async function buildNotificationMessage(
  title: string,
  summary: string,
  context: StreamNotificationContext,
) {
  const parts = [context.sessionTitle?.trim(), summary].filter(
    (value): value is string => !!value,
  );
  await sendSystemNotification(title, parts.join("\n") || undefined);
}

async function hasFocusedLocusWindow(): Promise<boolean> {
  try {
    if ((await Window.getFocusedWindow()) !== null) return true;
  } catch {
    if (document.hasFocus()) return true;
  }

  return hasFocusedUnityEmbedWindow();
}

async function hasFocusedUnityEmbedWindow(): Promise<boolean> {
  const snapshot = await getUnityEmbedFocusDebugSnapshot().catch(() => null);
  if (!snapshot?.ok || !snapshot.overlayVisible) return false;

  return snapshot.overlayInputFocused
    || (
      snapshot.overlayInputFocused == null
      && (snapshot.overlayForeground || snapshot.parentForeground)
    );
}

async function ensureNotificationPermission(): Promise<boolean> {
  if (permissionCheckInFlight) return permissionCheckInFlight;

  permissionCheckInFlight = (async () => {
    const alreadyGranted = await isPermissionGranted().catch(() => false);
    if (alreadyGranted) return true;
    if (permissionRequested) return false;

    permissionRequested = true;
    const permission = await requestPermission().catch(() => "denied");
    return permission === "granted";
  })();

  try {
    return await permissionCheckInFlight;
  } finally {
    permissionCheckInFlight = null;
  }
}

function isEventEnabled(event: NotifiableStreamEvent): boolean {
  const { state } = useDisplaySettings();
  if (!state.systemNotificationsEnabled) return false;

  switch (event.type) {
    case "done":
      return state.notifyOnChatDone;
    case "askUser":
      return state.notifyOnAskUser;
    case "error":
      return state.notifyOnChatError;
    case "toolConfirm":
      return state.notifyOnToolConfirm;
    case "knowledgeProposal":
      return state.notifyOnToolConfirm;
  }
}

export function resetSystemNotificationState() {
  recentNotificationKeys.clear();
}

export async function maybeNotifyStreamEvent(
  event: StreamEvent,
  context: StreamNotificationContext = {},
) {
  if (!isNotifiableStreamEvent(event)) return;
  if (!isEventEnabled(event)) return;

  const notificationKey = getNotificationKey(event);
  if (recentNotificationKeys.has(notificationKey)) return;

  if (await hasFocusedLocusWindow()) return;
  if (!(await ensureNotificationPermission())) return;

  switch (event.type) {
    case "done":
      await buildNotificationMessage(
        t("notifications.chatDoneTitle"),
        summarizeText(event.fullText) || t("notifications.chatDoneFallback"),
        context,
      );
      break;
    case "askUser":
      await buildNotificationMessage(
        t("notifications.askUserTitle"),
        summarizeText(event.question) || t("notifications.askUserFallback"),
        context,
      );
      break;
    case "error":
      await buildNotificationMessage(
        t("notifications.chatErrorTitle"),
        summarizeText(event.error.message) || t("notifications.chatErrorFallback"),
        context,
      );
      break;
    case "toolConfirm":
      await buildNotificationMessage(
        t("notifications.toolConfirmTitle"),
        summarizeToolConfirmDisplay(event.display) || t("notifications.toolConfirmFallback"),
        context,
      );
      break;
    case "knowledgeProposal":
      await buildNotificationMessage(
        t("notifications.knowledgeProposalTitle"),
        summarizeKnowledgeProposal(event),
        context,
      );
      break;
  }

  rememberNotificationKey(notificationKey);
}
