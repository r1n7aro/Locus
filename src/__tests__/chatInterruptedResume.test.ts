import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("interrupted chat resume", () => {
  it("turns the empty composer action into a resume button", () => {
    const composer = read("src/components/chat/ChatComposer.vue");
    const richInput = read("src/components/chat/RichChatInput.vue");
    const chatView = read("src/components/ChatView.vue");
    const workspace = read("src/components/ChatWorkspaceView.vue");

    expect(composer).toContain("!props.isStreaming && props.canResume && !props.canSend");
    expect(composer).toContain("emit(\"resume\")");
    expect(composer).toContain("chat-composer-resume-icon");
    expect(richInput).toContain(':can-resume="canResume"');
    expect(richInput).toContain("@resume=\"emit('resume')\"");
    expect(chatView).toContain(':can-resume="canResumeInterrupted"');
    expect(chatView).toContain("@resume=\"emit('resume')\"");
    expect(workspace).toContain(':can-resume-interrupted="chatStore.canResumeInterrupted"');
    expect(workspace).toContain('@resume="chatStore.resumeInterrupted"');
  });

  it("starts a hidden empty continuation turn and persists resume availability", () => {
    const chatStore = read("src/stores/chat.ts");
    const service = read("src/services/session.ts");
    const command = read("src-tauri/src/commands/session.rs");
    const sessionStore = read("src-tauri/src/session/store.rs");
    const tauriSetup = read("src-tauri/src/lib.rs");

    expect(chatStore).toContain("async function resumeInterrupted()");
    expect(chatStore).toMatch(/sessionService\.chat\(\{[\s\S]*?text: "",[\s\S]*?resume: true,/);
    expect(service).toContain('ipcInvoke<boolean>("get_session_resume_available", { sessionId })');
    expect(command).toContain("let initial_system_reminder = resume_requested.then");
    expect(command).toContain("let mut next_internal_system_reminder = initial_system_reminder;");
    expect(command).toContain('"session.resume_unavailable"');
    expect(sessionStore).toContain("pub fn session_resume_available");
    expect(sessionStore).toContain("WHERE run_id = ?1 AND event_type = 'userMessage'");
    expect(sessionStore).toContain("remove_internal_system_reminders_from_display");
    expect(tauriSetup).toContain("commands::get_session_resume_available");
  });
});
