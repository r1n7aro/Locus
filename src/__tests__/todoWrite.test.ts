import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { parseLegacyTodoWriteOutput, parseTodoWriteArguments } from "../composables/todoWrite";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("todowrite presentation", () => {
  it("normalizes tool arguments into displayable todos", () => {
    expect(parseTodoWriteArguments(JSON.stringify({
      todos: [
        { content: "done", status: "completed", priority: "high" },
        { content: "active", status: "in_progress" },
        { content: "invalid", status: "unknown", priority: "unknown" },
        { status: "pending" },
      ],
    }))).toEqual([
      { content: "done", status: "completed", priority: "high" },
      { content: "active", status: "in_progress", priority: "medium" },
      { content: "invalid", status: "pending", priority: "medium" },
    ]);
  });

  it("supports echoed results from older sessions", () => {
    expect(parseLegacyTodoWriteOutput([
      "1 todos (0 remaining)",
      '[{"content":"done","status":"completed","priority":"low"}]',
    ].join("\n"))).toEqual([
      { content: "done", status: "completed", priority: "low" },
    ]);
  });

  it("uses a dedicated todo list block without argument and output sections", () => {
    const toolBlock = read("src/components/ToolCallBlock.vue");
    const backend = read("src-tauri/src/agent/instance/mod.rs");

    expect(toolBlock).toContain("<TodoList");
    expect(toolBlock).toContain('v-if="isTodoWriteTool"');
    expect(toolBlock).toContain('v-if="!isTodoWriteTool && (toolCall.output !== undefined || toolCall.status === \'running\')"');
    expect(backend).toContain('Todos updated ({} total, {} remaining).');

    const todoWriteStart = backend.indexOf("fn execute_todowrite");
    const todoWriteEnd = backend.indexOf("pub(super) async fn clear_pending_knowledge_proposal", todoWriteStart);
    const todoWriteSource = backend.slice(todoWriteStart, todoWriteEnd);
    expect(todoWriteSource).not.toContain("to_string_pretty(&items)");
  });
});
