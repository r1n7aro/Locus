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
      source.indexOf("pub async fn get_agent_rendered_env_prompt"),
      source.indexOf("pub async fn create_session"),
    );

    expect(previewSection.match(/config: State<'_, Arc<AppConfig>>/g)).toHaveLength(3);
    expect(
      previewSection.match(
        /instance\.set_async_tasks_enabled\(config\.async_tasks_enabled\(\)\)/g,
      ),
    ).toHaveLength(3);
  });
});
