import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat compact route", () => {
  it("routes the slash compact action through the compact event", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");
    const chatView = read("src/components/ChatView.vue");
    const workspace = read("src/components/ChatWorkspaceView.vue");

    expect(richInput).toContain('(e: "compact"): void;');
    expect(richInput).toContain('emit("compact");');
    expect(richInput).not.toContain("getCompactInstruction");
    expect(chatView).toContain("compact: [];");
    expect(chatView).toContain('@compact="emit(\'compact\')"');
    expect(workspace).toContain('@compact="chatStore.compactSession"');
  });

  it("starts compact as a dedicated session run without adding a pending user message", () => {
    const chatStore = read("src/stores/chat.ts");

    const compactStart = chatStore.indexOf("async function compactSession()");
    const sendStart = chatStore.indexOf("async function sendMessage(");
    const cancelStart = chatStore.indexOf("async function cancelSession(");
    const compactBody = chatStore.slice(compactStart, cancelStart);

    expect(compactStart).toBeGreaterThan(sendStart);
    expect(compactBody).toContain('mode: "compact"');
    expect(compactBody).toContain('text: ""');
    expect(compactBody).not.toContain("messages.value.push");
  });

  it("uses the backend compact path for manual compact mode", () => {
    const agentInstance = read("src-tauri/src/agent/instance/mod.rs");

    const modeStart = agentInstance.indexOf('if initial_mode == "compact"');
    const compactCall = agentInstance.indexOf(".execute_auto_compact(", modeStart);
    const forceFlag = agentInstance.indexOf("true,", compactCall);
    const doneEvent = agentInstance.indexOf("StreamEvent::Done", compactCall);

    expect(modeStart).toBeGreaterThanOrEqual(0);
    expect(compactCall).toBeGreaterThan(modeStart);
    expect(forceFlag).toBeGreaterThan(compactCall);
    expect(forceFlag).toBeLessThan(doneEvent);
  });

  it("persists and emits compacted context usage after compact replaces messages", () => {
    const agentInstance = read("src-tauri/src/agent/instance/mod.rs");
    const streamEvents = read("src-tauri/src/commands/mod.rs");

    expect(agentInstance).toContain("async fn persist_compacted_context_usage(");
    expect(agentInstance).toContain(".persist_compacted_context_usage(store, system_parts, context_limit)");
    expect(agentInstance).toContain("context_tokens: compacted_context_tokens");
    expect(streamEvents).toContain("context_tokens: u32");
    expect(streamEvents).toContain("context_limit: u32");
  });

  it("warns with a banner when compaction reacts to a server context overflow", () => {
    const chatStore = read("src/stores/chat.ts");
    const streamEvents = read("src-tauri/src/commands/mod.rs");
    const agentInstance = read("src-tauri/src/agent/instance/mod.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(streamEvents).toContain("pub enum CompactTrigger");
    expect(streamEvents).toContain("trigger: Option<CompactTrigger>");
    expect(agentInstance).toContain("compact_trigger(force_compact, attempt_kind)");
    expect(agentInstance).toContain("REACTIVE_COMPACT_ATTEMPT_KIND,");
    expect(chatStore).toContain('event.type === "compactStart" && event.trigger === "reactive"');
    expect(chatStore).toContain('addNotice("warning", t("chat.transcript.reactiveCompactNotice")');
    expect(zh).toContain('"chat.transcript.reactiveCompactNotice"');
    expect(en).toContain('"chat.transcript.reactiveCompactNotice"');
  });

  it("renders compacted handoff messages as a transcript divider", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const store = read("src-tauri/src/session/store.rs");
    const zh = read("src/language/zh.json");

    expect(store).toContain("CONTEXT_COMPACTED_DISPLAY_MARKER");
    expect(store).toContain("redact_context_handoff_for_display");
    expect(transcript).toContain("isCompactMarkerGroup(group)");
    expect(transcript).toContain("chat-transcript-compact-marker-label");
    expect(zh).toContain('"chat.transcript.compacted": "上下文已压缩"');
  });
});
