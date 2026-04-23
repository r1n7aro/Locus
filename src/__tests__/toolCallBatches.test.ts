import { describe, expect, it } from "vitest";
import {
  buildMessageToolCalls,
  collectToolCallDisplayIds,
  collectToolCallDisplayMatchState,
  filterToolCallsByActiveIds,
  filterToolCallsByMatchState,
  mergeSequentialAssistantToolCalls,
  mergeToolCallDisplaysWithoutDuplicates,
  resolveToolCallInfosForRender,
  summarizeToolCallBatch,
} from "../composables/toolCallBatches";

import type { ChatMessage, ToolCallDisplay, ToolCallInfo } from "../types";

function makeToolCall(status: ToolCallDisplay["status"], id: string): ToolCallDisplay {
  return {
    id,
    name: "read",
    arguments: "{}",
    status,
  };
}

function makeToolInfo(id: string, name = "read"): ToolCallInfo {
  return {
    id,
    name,
    arguments: "{}",
  };
}

describe("toolCallBatches", () => {
  it("builds historical message tool calls with persisted output fallback", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "tc-1",
          name: "web_search",
          arguments: "{\"q\":\"unity\"}",
        },
      ],
    };

    expect(buildMessageToolCalls(message, { "tc-1": "cached output" })).toEqual([
      {
        id: "tc-1",
        name: "web_search",
        arguments: "{\"q\":\"unity\"}",
        status: "done",
        output: "cached output",
      },
    ]);
  });

  it("prefers server tool output for historical message tool calls", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "tc-2",
          name: "web_search",
          arguments: "{\"q\":\"agent\"}",
          serverToolOutput: "server output",
        },
      ],
    };

    expect(buildMessageToolCalls(message, { "tc-2": "cached output" })[0]?.output).toBe("server output");
  });

  it("preserves persisted outcomes for historical tool calls", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "tc-3",
          name: "write",
          arguments: "{\"path\":\"Assets/Player.cs\"}",
          outcome: "error",
        },
      ],
    };

    expect(buildMessageToolCalls(message, {})[0]?.status).toBe("error");
  });

  it("restores nested tool calls and recorded output from persisted history", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "task-1",
          name: "task",
          arguments: "{}",
          outcome: "done",
          nestedToolCalls: [
            {
              id: "read-1",
              name: "read",
              arguments: "{\"path\":\"Assets/Player.cs\"}",
              outcome: "done",
              recordedOutput: "class Player {}",
            },
          ],
        },
      ],
    };

    const toolCalls = buildMessageToolCalls(message, {});
    expect(toolCalls[0]?.nestedToolCalls?.[0]?.output).toBe("class Player {}");
    expect(toolCalls[0]?.nestedToolCalls?.[0]?.status).toBe("done");
  });

  it("collects nested active tool call ids for transcript de-duplication", () => {
    const activeToolCalls: ToolCallDisplay[] = [
      {
        id: "task-1",
        name: "task",
        arguments: "{}",
        status: "running",
        nestedToolCalls: [
          {
            id: "read-1",
            name: "read",
            arguments: "{}",
            status: "done",
          },
        ],
      },
    ];

    expect(Array.from(collectToolCallDisplayIds(activeToolCalls))).toEqual(["task-1", "read-1"]);
  });

  it("filters historical tool calls that are still present in active tool calls", () => {
    const filtered = filterToolCallsByActiveIds(
      [
        makeToolInfo("task-1", "task"),
        makeToolInfo("read-1"),
        makeToolInfo("grep-1", "grep"),
      ],
      new Set(["task-1", "read-1"]),
    );

    expect(filtered).toEqual([makeToolInfo("grep-1", "grep")]);
  });

  it("drops historical tool-only rounds after filtering when the active copy is already visible", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: filterToolCallsByActiveIds(
          [makeToolInfo("task-1", "task")],
          new Set(["task-1"]),
        ),
      },
      {
        id: "m2",
        content: "继续处理后续文件。",
      },
    ]);

    expect(merged).toHaveLength(2);
    expect(merged[0]?.displayToolCalls).toBeUndefined();
    expect(merged[1]?.displayToolCalls).toBeUndefined();
  });

  it("keeps resolved empty display tool calls hidden instead of falling back to persisted history", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: filterToolCallsByMatchState(
          [makeToolInfo("task-1", "task")],
          collectToolCallDisplayMatchState([makeToolCall("done", "task-1")]),
        ),
      },
    ]);

    expect(
      resolveToolCallInfosForRender({
        messageToolCalls: [makeToolInfo("task-1", "task")],
        displayToolCalls: merged[0]?.displayToolCalls,
      }),
    ).toBeUndefined();
  });

  it("falls back to persisted history only when no resolved display tool calls exist", () => {
    expect(
      resolveToolCallInfosForRender({
        messageToolCalls: [makeToolInfo("task-1", "task")],
      }),
    ).toEqual([makeToolInfo("task-1", "task")]);
  });

  it("filters historical tool calls that match the transient copy even when ids differ", () => {
    const hiddenState = collectToolCallDisplayMatchState([
      {
        id: "active-1",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        status: "done",
      },
    ]);

    const filtered = filterToolCallsByMatchState(
      [
        {
          id: "history-1",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        },
        {
          id: "history-2",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoB.cs\"}",
        },
      ],
      hiddenState,
    );

    expect(filtered).toEqual([
      {
        id: "history-2",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoB.cs\"}",
      },
    ]);
  });

  it("merges transient and promoted history tool calls without duplicating semantic matches", () => {
    const merged = mergeToolCallDisplaysWithoutDuplicates(
      [
        {
          id: "history-1",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
          status: "done",
        },
      ],
      [
        {
          id: "active-1",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
          status: "done",
        },
        {
          id: "active-2",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoB.cs\"}",
          status: "done",
        },
      ],
    );

    expect(merged.map((toolCall) => toolCall.id)).toEqual(["history-1", "active-2"]);
  });

  it("deduplicates read tool calls when path aliases differ across transient and history copies", () => {
    const filtered = filterToolCallsByMatchState(
      [
        {
          id: "history-1",
          name: "read",
          arguments: "{\"filePath\":\"Assets\\\\Scripts\\\\TestMonoA.cs\"}",
        },
      ],
      collectToolCallDisplayMatchState([
        {
          id: "active-1",
          name: "read",
          arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
          status: "done",
        },
      ]),
    );

    expect(filtered).toBeUndefined();
  });

  it("deduplicates edit tool calls when camelCase and snake_case aliases mix", () => {
    const merged = mergeToolCallDisplaysWithoutDuplicates(
      [
        {
          id: "history-1",
          name: "edit",
          arguments: "{\"file_path\":\"Assets/Test.cs\",\"old_string\":\"a\",\"new_string\":\"b\",\"replace_all\":true}",
          status: "done",
        },
      ],
      [
        {
          id: "active-1",
          name: "edit",
          arguments: "{\"filePath\":\"Assets/Test.cs\",\"oldString\":\"a\",\"newString\":\"b\",\"replaceAll\":true}",
          status: "done",
        },
      ],
    );

    expect(merged.map((toolCall) => toolCall.id)).toEqual(["history-1"]);
  });

  it("collapses completed tool batches when compact mode is enabled", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("done", "tc-2")],
      true,
    );

    expect(batch.canCollapse).toBe(true);
    expect(batch.total).toBe(2);
    expect(batch.doneCount).toBe(2);
  });

  it("keeps completed tool batches expanded when compact mode is disabled", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("done", "tc-2")],
      false,
    );

    expect(batch.canCollapse).toBe(false);
  });

  it("keeps single tool batches expanded", () => {
    const batch = summarizeToolCallBatch([makeToolCall("done", "tc-1")], true);

    expect(batch.canCollapse).toBe(false);
  });

  it("keeps running streaming tool batches expanded", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("running", "tc-2")],
      true,
    );

    expect(batch.canCollapse).toBe(false);
    expect(batch.runningCount).toBe(1);
  });

  it("keeps failed tool batches expanded", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("error", "tc-2")],
      true,
    );

    expect(batch.canCollapse).toBe(false);
    expect(batch.errorCount).toBe(1);
  });

  it("keeps interrupted tool batches expanded", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("interrupted", "tc-2")],
      true,
    );

    expect(batch.canCollapse).toBe(false);
    expect(batch.interruptedCount).toBe(1);
  });

  it("applies the same rule to nested tool batches", () => {
    const parent: ToolCallDisplay = {
      id: "parent",
      name: "task",
      arguments: "{}",
      status: "done",
      nestedToolCalls: [makeToolCall("done", "tc-1"), makeToolCall("done", "tc-2")],
    };

    const batch = summarizeToolCallBatch(parent.nestedToolCalls ?? [], true);

    expect(batch.canCollapse).toBe(true);
  });

  it("merges tool-only assistant rounds into the following visible round", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: [makeToolInfo("tc-ask", "ask_user_question")],
      },
      {
        id: "m2",
        content: "",
        toolCalls: [makeToolInfo("tc-list", "list")],
      },
      {
        id: "m3",
        content: "已并行读取项目目录下 4 个现有文件。",
        toolCalls: [
          makeToolInfo("tc-read-1"),
          makeToolInfo("tc-read-2"),
          makeToolInfo("tc-read-3"),
          makeToolInfo("tc-read-4"),
        ],
      },
    ]);

    expect(merged).toHaveLength(1);
    expect(merged[0]?.id).toBe("m3");
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual([
      "tc-ask",
      "tc-list",
      "tc-read-1",
      "tc-read-2",
      "tc-read-3",
      "tc-read-4",
    ]);
  });

  it("keeps visible assistant rounds separate", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "先确认范围。",
        toolCalls: [makeToolInfo("tc-ask", "ask_user_question")],
      },
      {
        id: "m2",
        content: "目录里只有 4 个文件。",
        toolCalls: [makeToolInfo("tc-list", "list")],
      },
    ]);

    expect(merged).toHaveLength(2);
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-ask"]);
    expect(merged[1]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-list"]);
  });

  it("keeps trailing tool-only assistant rounds separate before text arrives", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: [makeToolInfo("tc-1")],
      },
      {
        id: "m2",
        content: "",
        toolCalls: [makeToolInfo("tc-2")],
      },
      {
        id: "m3",
        content: "",
        toolCalls: [makeToolInfo("tc-3")],
      },
    ]);

    expect(merged).toHaveLength(3);
    expect(merged.map((item) => item.id)).toEqual(["m1", "m2", "m3"]);
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-1"]);
    expect(merged[1]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-2"]);
    expect(merged[2]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-3"]);
  });

  it("waits for actual text instead of thinking-only state before merging", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: [makeToolInfo("tc-1")],
      },
      {
        id: "m2",
        content: "",
        thinkingContent: "thinking",
        toolCalls: [makeToolInfo("tc-2")],
      },
      {
        id: "m3",
        content: "最终答复",
      },
    ]);

    expect(merged).toHaveLength(1);
    expect(merged[0]?.id).toBe("m3");
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-1", "tc-2"]);
  });
});
