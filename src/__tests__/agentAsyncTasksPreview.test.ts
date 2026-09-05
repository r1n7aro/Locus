import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");

describe("Agent async task preview", () => {
  it("applies the persisted experimental gate to every Agent preview instance", () => {
    const source = readFileSync(
      resolve(root, "src-tauri/src/commands/session.rs"),
      "utf8",
    );
    const previewSection = source.slice(
      source.indexOf("async fn workspace_agent_preview_instance"),
      source.indexOf("pub async fn create_session"),
    );

    expect(previewSection.match(/config: State<'_, Arc<AppConfig>>/g)).toHaveLength(6);
    expect(
      previewSection.match(
        /instance\.set_async_tasks_enabled\(config\.async_tasks_enabled\(\)\)/g,
      ),
    ).toHaveLength(4);
  });

  it("delivers notify completion automatically without status polling", () => {
    const asyncTasks = readFileSync(
      resolve(root, "src-tauri/src/async_tasks.rs"),
      "utf8",
    );
    const agent = readFileSync(
      resolve(root, "src-tauri/src/agent/instance/mod.rs"),
      "utf8",
    );
    const session = readFileSync(
      resolve(root, "src-tauri/src/commands/session.rs"),
      "utf8",
    );
    const bashTool = readFileSync(resolve(root, "tools/bash.json"), "utf8");

    expect(asyncTasks).toContain("Use the result below to continue the task");
    expect(asyncTasks).toContain("await locus.get_task_status(task_id)");
    expect(agent).toContain("manager.finish_without_notification(&task_id, &result)");
    expect(agent).toContain("manager.enqueue_completion_notification(&snapshot)");
    expect(agent).toContain("manager.deliver_notifications(&self.session_id, store)");
    const injection = agent.indexOf("let injected_async_notifications =");
    const nextPromptRead = agent.indexOf(
      "store.get_messages_for_prompt(&self.session_id)?",
      injection,
    );
    expect(injection).toBeGreaterThan(0);
    expect(nextPromptRead).toBeGreaterThan(injection);
    expect(session).toContain("async_tasks.take_notifications_and_pending(&sid_clone)");
    expect(session).toContain("next_internal_system_reminder = async_reminder");
    expect(bashTool).toContain("notify automatically delivers the final result");
  });
});
