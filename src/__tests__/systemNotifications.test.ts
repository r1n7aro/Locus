import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DisplaySettings } from "../composables/useDisplaySettings";
import {
  maybeNotifyStreamEvent,
  resetSystemNotificationState,
} from "../services/systemNotifications";

const translations = {
  "notifications.chatDoneTitle": "对话已完成",
  "notifications.askUserTitle": "需要你的输入",
  "notifications.chatErrorTitle": "对话出错",
  "notifications.toolConfirmTitle": "需要确认",
  "notifications.knowledgeProposalTitle": "知识提案待处理",
  "notifications.chatDoneFallback": "回复已生成",
  "notifications.askUserFallback": "等待你的输入",
  "notifications.chatErrorFallback": "运行失败",
  "notifications.toolConfirmFallback": "有工具操作等待确认",
  "notifications.knowledgeProposalFallback": "有新的知识提案等待处理",
  "notifications.knowledgeProposalSingle": "{0}",
  "notifications.knowledgeProposalMultiple": "{0} 项知识更新建议",
} as Record<string, string>;

function createDisplayState(): DisplaySettings {
  return {
    todoAutoOpen: true,
    changesAutoOpen: true,
    changesAutoClose: true,
    rightAlignUserMessages: false,
    compactToolCalls: true,
    mergeGitTreeStatusIcon: true,
    systemNotificationsEnabled: true,
    notifyOnChatDone: true,
    notifyOnAskUser: true,
    notifyOnChatError: true,
    notifyOnToolConfirm: true,
    fonts: {
      ui: "",
      prose: "",
      monoInline: "",
      monoBlock: "",
      monoEditor: "",
    },
  };
}

const notificationMocks = vi.hoisted(() => ({
  isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(),
}));

const systemMocks = vi.hoisted(() => ({
  sendSystemNotification: vi.fn(),
}));

const windowMocks = vi.hoisted(() => ({
  getFocusedWindow: vi.fn(),
}));

const displayState = createDisplayState();

vi.mock("@tauri-apps/plugin-notification", () => notificationMocks);
vi.mock("../services/system", () => systemMocks);
vi.mock("@tauri-apps/api/window", () => ({
  Window: windowMocks,
}));
vi.mock("../composables/useDisplaySettings", () => ({
  useDisplaySettings: () => ({
    state: displayState,
  }),
}));
vi.mock("../i18n", () => ({
  t: (key: string, ...args: (string | number)[]) => {
    const message = translations[key] ?? key;
    if (args.length === 0) return message;
    return message.replace(/\{(\d+)\}/g, (_, index) => {
      const argIndex = Number(index);
      return argIndex < args.length ? String(args[argIndex]) : `{${index}}`;
    });
  },
}));

describe("systemNotifications", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetSystemNotificationState();

    displayState.todoAutoOpen = true;
    displayState.changesAutoOpen = true;
    displayState.changesAutoClose = true;
    displayState.compactToolCalls = true;
    displayState.mergeGitTreeStatusIcon = true;
    displayState.systemNotificationsEnabled = true;
    displayState.notifyOnChatDone = true;
    displayState.notifyOnAskUser = true;
    displayState.notifyOnChatError = true;
    displayState.notifyOnToolConfirm = true;

    notificationMocks.isPermissionGranted.mockResolvedValue(true);
    notificationMocks.requestPermission.mockResolvedValue("granted");
    windowMocks.getFocusedWindow.mockResolvedValue(null);
  });

  it("sends a completion notification when no app window is focused", async () => {
    await maybeNotifyStreamEvent(
      {
        type: "done",
        runId: "run-1",
        sessionId: "session-1",
        messageId: "message-1",
        fullText: "已完成输出",
      },
      { sessionTitle: "登录修复" },
    );

    expect(systemMocks.sendSystemNotification).toHaveBeenCalledWith(
      "对话已完成",
      "登录修复\n已完成输出",
    );
  });

  it("skips notifications while any Locus window is focused", async () => {
    windowMocks.getFocusedWindow.mockResolvedValue({ label: "main" });

    await maybeNotifyStreamEvent({
      type: "error",
      runId: "run-2",
      sessionId: "session-1",
      error: {
        code: "failed",
        message: "stream failed",
        retryable: false,
        severity: "error",
      },
    });

    expect(systemMocks.sendSystemNotification).not.toHaveBeenCalled();
  });

  it("skips notifications when the master toggle is disabled", async () => {
    displayState.systemNotificationsEnabled = false;

    await maybeNotifyStreamEvent({
      type: "askUser",
      runId: "run-3",
      sessionId: "session-1",
      questionId: "question-1",
      toolCallId: "tool-1",
      question: "请确认目录",
      options: [],
    });

    expect(systemMocks.sendSystemNotification).not.toHaveBeenCalled();
  });

  it("deduplicates repeated ask-user notifications", async () => {
    await maybeNotifyStreamEvent({
      type: "askUser",
      runId: "run-4",
      sessionId: "session-1",
      questionId: "question-2",
      toolCallId: "tool-1",
      question: "请选择路径",
      options: [],
    });
    await maybeNotifyStreamEvent({
      type: "askUser",
      runId: "run-4",
      sessionId: "session-1",
      questionId: "question-2",
      toolCallId: "tool-1",
      question: "请选择路径",
      options: [],
    });

    expect(systemMocks.sendSystemNotification).toHaveBeenCalledTimes(1);

    resetSystemNotificationState();
    await maybeNotifyStreamEvent({
      type: "askUser",
      runId: "run-4",
      sessionId: "session-1",
      questionId: "question-2",
      toolCallId: "tool-1",
      question: "请选择路径",
      options: [],
    });

    expect(systemMocks.sendSystemNotification).toHaveBeenCalledTimes(2);
  });

  it("obeys per-event notification toggles", async () => {
    displayState.notifyOnToolConfirm = false;

    await maybeNotifyStreamEvent({
      type: "toolConfirm",
      runId: "run-5",
      sessionId: "session-1",
      questionId: "question-3",
      toolCallId: "tool-2",
      display: {
        kind: "basic",
        toolName: "edit",
        arguments: "{\"path\":\"a.ts\"}",
      },
    });

    expect(systemMocks.sendSystemNotification).not.toHaveBeenCalled();
  });

  it("sends a knowledge proposal notification with the proposal target", async () => {
    await maybeNotifyStreamEvent(
      {
        type: "knowledgeProposal",
        runId: "run-6",
        sessionId: "session-1",
        message: {
          id: "message-6",
          role: "assistant",
          content: "",
          createdAt: Date.now(),
          knowledgeProposal: {
            proposalId: "proposal-1",
            status: "pending",
            confidence: 0.92,
            verify: "required",
            estTokens: 128,
            items: [
              {
                kind: "knowledge",
                mode: "update_source",
                target: "reference/api/auth.md",
                draft: "draft",
              },
            ],
            createdAt: Date.now(),
            updatedAt: Date.now(),
          },
        },
      },
      { sessionTitle: "Auth Docs" },
    );

    expect(systemMocks.sendSystemNotification).toHaveBeenCalledWith(
      "知识提案待处理",
      "Auth Docs\nreference/api/auth.md",
    );
  });

  it("deduplicates repeated knowledge proposal notifications by proposal id", async () => {
    const proposalEvent = {
      type: "knowledgeProposal" as const,
      runId: "run-7",
      sessionId: "session-1",
      message: {
        id: "message-7",
        role: "assistant" as const,
        content: "",
        createdAt: Date.now(),
        knowledgeProposal: {
          proposalId: "proposal-2",
          status: "pending" as const,
          confidence: 0.8,
          verify: "none" as const,
          estTokens: 64,
          items: [
            {
              kind: "memory" as const,
              mode: "replace" as const,
              target: "memory/project_understanding.md",
              draft: "draft",
            },
            {
              kind: "knowledge" as const,
              mode: "update_source" as const,
              target: "design/flow.md",
              draft: "draft",
            },
          ],
          createdAt: Date.now(),
          updatedAt: Date.now(),
        },
      },
    };

    await maybeNotifyStreamEvent(proposalEvent);
    await maybeNotifyStreamEvent(proposalEvent);

    expect(systemMocks.sendSystemNotification).toHaveBeenCalledTimes(1);
    expect(systemMocks.sendSystemNotification).toHaveBeenCalledWith(
      "知识提案待处理",
      "2 项知识更新建议",
    );
  });
});
