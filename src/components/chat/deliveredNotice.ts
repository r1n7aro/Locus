import { t } from "../../i18n";
import type { ChatMessage } from "../../types";

/**
 * A background task's result or an agent's message delivered to the agent.
 * The backend stores each as an internal user row (`async-result:<task>`,
 * `agent-message:<id>`) whose reminder the model reads; the transcript shows
 * a one-line system notice instead, never a user bubble.
 */
export interface DeliveredNotice {
  kind: "task" | "message";
  /** The task's tool: `Agent` for a Claude subagent, `subagent` for a Locus one. */
  toolName?: string;
  description?: string;
  status?: string;
  /** The agent that sent the message. */
  from?: string;
}

const TASK_RESULT = /^Async task \S+ \(([^)]*)\) attempt \d+ finished with status (\w+)\./m;
const TASK_DESCRIPTION = /^Task: (.*)$/m;
const AGENT_TOOLS = new Set(["Agent", "subagent"]);
const STATUSES = new Set(["completed", "failed", "cancelled"]);

export function deliveredNotice(message: ChatMessage): DeliveredNotice | null {
  if (message.role !== "user" || message.content.trim()) return null;
  const reminder = message.promptSuffix ?? "";
  if (message.id.startsWith("agent-message:")) {
    const data = reminder.slice(reminder.indexOf("{"), reminder.lastIndexOf("}") + 1);
    try {
      const parsed = JSON.parse(data) as { from?: unknown };
      return { kind: "message", from: typeof parsed.from === "string" ? parsed.from : "" };
    } catch {
      return { kind: "message", from: "" };
    }
  }
  if (!message.id.startsWith("async-result:")) return null;
  const result = TASK_RESULT.exec(reminder);
  if (!result) return null;
  return {
    kind: "task",
    toolName: result[1],
    status: result[2],
    description: TASK_DESCRIPTION.exec(reminder)?.[1]?.trim() || undefined,
  };
}

export function deliveredNoticeLabel(notice: DeliveredNotice): string {
  if (notice.kind === "message") {
    return notice.from ? t("chat.deliveredNotice.message", notice.from) : t("chat.deliveredNotice.messageUnnamed");
  }
  const subject = AGENT_TOOLS.has(notice.toolName ?? "") ? "agent" : "task";
  const status = STATUSES.has(notice.status ?? "") ? notice.status! : "completed";
  return t(`chat.deliveredNotice.${subject}.${status}`, notice.description || notice.toolName || "");
}
