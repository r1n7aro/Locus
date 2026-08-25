import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("subagent session controls", () => {
  it("keeps the shared composer visible with runtime context actions", () => {
    const chatView = read("src/components/ChatView.vue");
    const inputArea = chatView.slice(
      chatView.indexOf('class="input-area"') - 80,
      chatView.indexOf('</RichChatInput>') + '</RichChatInput>'.length,
    );

    expect(inputArea).not.toContain('v-if="!isViewingSubagent"');
    expect(inputArea).toContain(':allow-runtime-compact="!isViewingSubagent"');
    expect(inputArea).toContain('@export-context="emit(\'exportSessionContext\'');
    expect(inputArea).toContain('@review-context="emit(\'reviewSessionContext\'');
    expect(inputArea).toContain('@cancel="emit(\'cancel\')"');
    expect(chatView).toContain(
      'isViewingSubagent.value || chatInputSettings.runningSendMode === "insert"',
    );
  });

  it("registers a child run as an independently controllable active task", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const runSubagent = agent.slice(
      agent.indexOf("async fn run_subagent"),
      agent.indexOf("fn suppressed_subagent_tool_result"),
    );

    expect(runSubagent).toContain("tokio::sync::watch::channel(false)");
    expect(runSubagent).toContain("crate::ActiveTaskHandle");
    expect(runSubagent).toContain("partial_assistant: child_partial_assistant");
    expect(runSubagent).toContain("subagent_interrupted_result(&child_messages)");
  });
});
