import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("workbench session title generation", () => {
  it("leaves a new generic chat title unset for the backend title pipeline", () => {
    const sessionEditor = read("src/components/workbench/WorkbenchSessionEditor.vue");
    const embeddedSession = read("src/composables/useEmbeddedChatSession.ts");
    const sessionCommand = read("src-tauri/src/commands/session.rs");

    expect(sessionEditor).toContain("sessionTitle: null");
    expect(sessionEditor).not.toContain("sessionTitle: computed(() => props.editor.title)");
    expect(embeddedSession).toContain("sessionTitle: toValue(options.sessionTitle) ?? null");
    expect(sessionCommand).toContain("explicit_session_title.is_none()");
    expect(sessionCommand).toContain("prepare_session_title_prompt(&text)");
    expect(sessionCommand).toContain("explicit_session_title.unwrap_or_else(|| text.chars().take(20).collect())");
  });
});
