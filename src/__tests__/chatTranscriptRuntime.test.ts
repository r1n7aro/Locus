// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, nextTick } from "vue";
import { describe, expect, it } from "vitest";
import ChatTranscript from "../components/chat/ChatTranscript.vue";
import type { ChatMessage, ToolCallDisplay, ToolCallInfo } from "../types";

describe("chat transcript runtime", () => {
  it("renders trailing history tools while a different live tool call is active", async () => {
    const historyTool: ToolCallInfo = {
      id: "history-tool",
      name: "read",
      arguments: JSON.stringify({ filePath: "Assets/History.cs" }),
      order: 1,
      outcome: "done",
      recordedOutput: "history output",
    };
    const messages: ChatMessage[] = [
      {
        id: "user-message",
        role: "user",
        content: "Inspect the project",
        createdAt: 1,
      },
      {
        id: "history-message",
        role: "assistant",
        content: "",
        createdAt: 2,
        toolCalls: [historyTool],
        renderParts: [{
          kind: "toolCall",
          id: "history-tool-part",
          order: { runId: "history-run", seq: 1 },
          toolCall: historyTool,
        }],
      },
    ];
    const activeToolCalls: ToolCallDisplay[] = [{
      id: "live-tool",
      name: "read",
      arguments: JSON.stringify({ filePath: "Assets/Live.cs" }),
      status: "running",
      order: 2,
    }];
    const runtimeErrors: unknown[] = [];
    const host = document.createElement("div");
    const app = createApp(ChatTranscript, {
      messages,
      streamingText: "",
      isStreaming: true,
      isThinking: false,
      activeToolCalls,
      variant: "session",
      sessionKey: "runtime-cycle-regression",
    });
    app.config.errorHandler = (error) => runtimeErrors.push(error);
    app.use(createPinia());

    app.mount(host);
    await nextTick();

    expect(runtimeErrors).toEqual([]);
    expect(host.querySelector(".chat-transcript-content")).not.toBeNull();
    expect(host.querySelectorAll(".chat-transcript-message").length).toBeGreaterThan(0);
    expect(host.textContent).toContain("Inspect the project");

    app.unmount();
  });
});
