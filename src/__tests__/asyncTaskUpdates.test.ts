import { describe, expect, it } from "vitest";
import { applyAsyncTaskUpdateToMessages } from "../composables/asyncTaskUpdates";
import type { AsyncTaskUpdatedEvent, ChatMessage, ToolCallInfo } from "../types";

const queuedToolCall: ToolCallInfo = {
  id: "tool-1",
  name: "bash",
  arguments: JSON.stringify({ command: "build", async: "notify" }),
  outcome: "done",
  recordedOutput: "Async task: id=task-1 status=queued",
};

const message: ChatMessage = {
  id: "assistant-1",
  role: "assistant",
  content: "",
  createdAt: 1,
  toolCalls: [queuedToolCall],
  renderParts: [{
    kind: "toolCall",
    id: "part-1",
    order: { runId: "run-1", seq: 1 },
    toolCall: queuedToolCall,
  }],
};

function update(
  status: AsyncTaskUpdatedEvent["status"],
  output: string,
): AsyncTaskUpdatedEvent {
  return {
    sessionId: "session-1",
    assistantMessageId: "assistant-1",
    toolCallId: "tool-1",
    taskId: "task-1",
    toolName: "bash",
    status,
    output,
  };
}

describe("async task frontend updates", () => {
  it("keeps a background tool running while replacing the queued text with live output", () => {
    const [next] = applyAsyncTaskUpdateToMessages([message], update("running", "line 1\n"));

    expect(next.toolCalls?.[0]).toMatchObject({
      recordedOutput: "line 1\n",
    });
    expect(next.toolCalls?.[0]?.outcome).toBeUndefined();
    const renderTool = next.renderParts?.[0];
    expect(renderTool?.kind).toBe("toolCall");
    if (renderTool?.kind === "toolCall") {
      expect(renderTool.toolCall.recordedOutput).toBe("line 1\n");
      expect(renderTool.toolCall.outcome).toBeUndefined();
    }
  });

  it("finalizes the historical tool block with the completed output", () => {
    const [next] = applyAsyncTaskUpdateToMessages(
      [message],
      update("completed", "Exit code: 0\nline 1\n"),
    );

    expect(next.toolCalls?.[0]).toMatchObject({
      outcome: "done",
      recordedOutput: "Exit code: 0\nline 1\n",
    });
  });
});
