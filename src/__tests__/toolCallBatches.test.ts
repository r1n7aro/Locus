import { describe, expect, it } from "vitest";
import {
  areToolCallDisplaysCoveredByMatchState,
  buildMessageToolCalls,
  collectToolCallDisplayIds,
  collectToolCallDisplayIdMatchState,
  collectToolCallDisplayMatchState,
  filterToolCallsByConsumableMatchState,
  filterToolCallsByActiveIds,
  filterToolCallsByMatchState,
  firstToolCallRenderOrder,
  hasVisibleTextPartAfterToolCalls,
  lastToolCallRenderOrder,
  mergeSequentialAssistantToolCalls,
  mergeToolCallDisplaysWithoutDuplicates,
  resolveToolCallInfosForRender,
  splitToolCallsByRenderOrder,
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

  it("renders meta tool_call as the target tool", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "tc-meta",
          name: "tool_call",
          arguments: JSON.stringify({
            toolName: "web_fetch",
            arguments: { url: "https://example.com" },
          }),
        },
      ],
    };

    expect(buildMessageToolCalls(message, { "tc-meta": "fetched" })).toEqual([
      {
        id: "tc-meta",
        name: "web_fetch",
        arguments: "{\"url\":\"https://example.com\"}",
        status: "done",
        output: "fetched",
      },
    ]);
  });

  it("attaches persisted tool result images to historical tool calls", () => {
    const image = { data: "iVBORw0KGgo=", mimeType: "image/png" };
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "tc-image",
          name: "unity_capture_viewport",
          arguments: "{\"target\":\"scene\"}",
        },
      ],
    };

    expect(buildMessageToolCalls(message, { "tc-image": "{\"image\":\"attached\"}" }, { "tc-image": [image] })).toEqual([
      {
        id: "tc-image",
        name: "unity_capture_viewport",
        arguments: "{\"target\":\"scene\"}",
        status: "done",
        output: "{\"image\":\"attached\"}",
        images: [image],
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

  it("preserves persisted render order for historical message tool calls", () => {
    const message: Pick<ChatMessage, "toolCalls"> = {
      toolCalls: [
        {
          id: "tc-ordered",
          name: "read",
          arguments: "{}",
          order: 4,
        },
      ],
    };

    expect(buildMessageToolCalls(message, {})[0]?.order).toBe(4);
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

  it("uses the earliest nested persisted order for a tool block", () => {
    const toolCalls: ToolCallDisplay[] = [
      {
        id: "task-1",
        name: "task",
        arguments: "{}",
        status: "done",
        nestedToolCalls: [
          {
            id: "read-1",
            name: "read",
            arguments: "{}",
            status: "done",
            order: 2,
          },
        ],
      },
    ];

    expect(firstToolCallRenderOrder(toolCalls)).toBe(2);
  });

  it("uses the latest nested persisted order for handoff collapse boundaries", () => {
    const toolCalls: ToolCallDisplay[] = [
      {
        id: "task-1",
        name: "task",
        arguments: "{}",
        status: "done",
        order: 3,
        nestedToolCalls: [
          {
            id: "read-1",
            name: "read",
            arguments: "{}",
            status: "done",
            order: 5,
          },
        ],
      },
      {
        id: "grep-1",
        name: "grep",
        arguments: "{}",
        status: "done",
        order: 4,
      },
    ];

    expect(lastToolCallRenderOrder(toolCalls)).toBe(5);
  });

  it("detects only body text rendered after the handoff tools", () => {
    const toolCalls: ToolCallDisplay[] = [
      { ...makeToolCall("done", "tc-1"), order: 3 },
      { ...makeToolCall("done", "tc-2"), order: 4 },
    ];

    expect(hasVisibleTextPartAfterToolCalls([
      {
        kind: "text",
        id: "before",
        order: { runId: "run-1", seq: 2 },
        content: "工具前的正文",
      },
    ], toolCalls)).toBe(false);

    expect(hasVisibleTextPartAfterToolCalls([
      {
        kind: "text",
        id: "after",
        order: { runId: "run-1", seq: 5 },
        content: "工具后的正文",
      },
    ], toolCalls)).toBe(true);
  });

  it("splits ordered tool groups around non-tool render boundaries", () => {
    const segments = splitToolCallsByRenderOrder(
      [
        { ...makeToolCall("done", "tc-before"), order: 2 },
        { ...makeToolCall("done", "tc-after-a"), order: 4 },
        { ...makeToolCall("done", "tc-after-b"), order: 5 },
      ],
      { fallbackOrder: 20, boundaryOrders: [3] },
    );

    expect(segments.map((segment) => segment.toolCalls.map((toolCall) => toolCall.id))).toEqual([
      ["tc-before"],
      ["tc-after-a", "tc-after-b"],
    ]);
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

  it("keeps prior identical tools visible while a new active tool is running", () => {
    const hiddenState = collectToolCallDisplayIdMatchState([
      {
        id: "active-recompile",
        name: "unity_recompile",
        arguments: "{\"project_path\":\"F:/Unity/Game\",\"editor_status\":\"editing\"}",
        status: "running",
      },
    ]);

    const priorRecompile = {
      id: "history-recompile",
      name: "unity_recompile",
      arguments: "{\"project_path\":\"F:/Unity/Game\",\"editor_status\":\"editing\"}",
    };

    expect(filterToolCallsByConsumableMatchState([priorRecompile], hiddenState)).toEqual([
      priorRecompile,
    ]);
    expect(filterToolCallsByConsumableMatchState([
      {
        ...priorRecompile,
        id: "active-recompile",
      },
    ], hiddenState)).toBeUndefined();
  });

  it("consumes transient semantic matches once across a transcript tail", () => {
    const hiddenState = collectToolCallDisplayMatchState([
      {
        id: "active-1",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        status: "done",
      },
    ]);
    const repeatedHistoryCall = {
      id: "history-1",
      name: "read",
      arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
    };

    expect(filterToolCallsByConsumableMatchState([repeatedHistoryCall], hiddenState)).toBeUndefined();
    expect(filterToolCallsByConsumableMatchState([
      {
        ...repeatedHistoryCall,
        id: "history-2",
      },
    ], hiddenState)).toEqual([
      {
        ...repeatedHistoryCall,
        id: "history-2",
      },
    ]);
  });

  it("consumes an id match together with its semantic fingerprint", () => {
    const hiddenState = collectToolCallDisplayMatchState([
      {
        id: "tc-1",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        status: "done",
      },
    ]);

    expect(filterToolCallsByConsumableMatchState([
      {
        id: "tc-1",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
      },
    ], hiddenState)).toBeUndefined();
    expect(filterToolCallsByConsumableMatchState([
      {
        id: "history-2",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
      },
    ], hiddenState)).toEqual([
      {
        id: "history-2",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
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

  it("checks whether a historical tool segment is fully covered by a retained match state", () => {
    const retainedState = collectToolCallDisplayMatchState([
      {
        id: "handoff-read",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        status: "done",
      },
      {
        id: "handoff-grep",
        name: "grep",
        arguments: "{\"path\":\"Assets/Scripts\"}",
        status: "done",
      },
    ]);

    expect(areToolCallDisplaysCoveredByMatchState([
      {
        id: "history-read",
        name: "read",
        arguments: "{\"filePath\":\"Assets\\\\Scripts\\\\TestMonoA.cs\"}",
        status: "done",
      },
      {
        id: "history-grep",
        name: "grep",
        arguments: "{\"path\":\"Assets/Scripts\"}",
        status: "done",
      },
    ], retainedState)).toBe(true);

    expect(areToolCallDisplaysCoveredByMatchState([
      {
        id: "history-read",
        name: "read",
        arguments: "{\"path\":\"Assets/Scripts/TestMonoA.cs\"}",
        status: "done",
      },
      {
        id: "history-extra",
        name: "list",
        arguments: "{\"path\":\"Assets\"}",
        status: "done",
      },
    ], retainedState)).toBe(false);
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

  it("collapses terminal tool batches with failed calls", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("error", "tc-2")],
      true,
    );

    expect(batch.canCollapse).toBe(true);
    expect(batch.errorCount).toBe(1);
  });

  it("collapses terminal tool batches with interrupted calls", () => {
    const batch = summarizeToolCallBatch(
      [makeToolCall("done", "tc-1"), makeToolCall("interrupted", "tc-2")],
      true,
    );

    expect(batch.canCollapse).toBe(true);
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
    expect(merged[0]?.displayToolCallsBeforeContent?.map((toolCall) => toolCall.id)).toEqual([
      "tc-ask",
      "tc-list",
    ]);
    expect(merged[0]?.displayToolCallsAfterContent?.map((toolCall) => toolCall.id)).toEqual([
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
    expect(merged[0]?.displayToolCallsAfterContent?.map((toolCall) => toolCall.id)).toEqual(["tc-ask"]);
    expect(merged[1]?.displayToolCallsAfterContent?.map((toolCall) => toolCall.id)).toEqual(["tc-list"]);
  });

  it("keeps trailing tool-only assistant rounds in one append-only list before text arrives", () => {
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

    expect(merged).toHaveLength(1);
    expect(merged[0]?.id).toBe("m1");
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-1", "tc-2", "tc-3"]);
  });

  it("keeps proposal-bearing tool rounds on their own render item", () => {
    const merged = mergeSequentialAssistantToolCalls([
      {
        id: "m1",
        content: "",
        toolCalls: [makeToolInfo("tc-proposal", "knowledge_edit")],
        attachedKnowledgeProposalCount: 1,
      },
      {
        id: "m2",
        content: "",
        toolCalls: [makeToolInfo("tc-read")],
      },
    ]);

    expect(merged).toHaveLength(2);
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-proposal"]);
    expect(merged[1]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-read"]);
  });

  it("keeps thinking-only tool rounds separate so their thinking block can render", () => {
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

    expect(merged).toHaveLength(3);
    expect(merged[0]?.id).toBe("m1");
    expect(merged[0]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-1"]);
    expect(merged[1]?.id).toBe("m2");
    expect(merged[1]?.thinkingContent).toBe("thinking");
    expect(merged[1]?.displayToolCalls?.map((toolCall) => toolCall.id)).toEqual(["tc-2"]);
    expect(merged[2]?.id).toBe("m3");
  });
});
