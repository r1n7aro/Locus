// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, h, nextTick, reactive } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import ChatTranscript from "../components/chat/ChatTranscript.vue";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { t } from "../i18n";
import type { AssistantRenderPart, ChatMessage, ClaudeCodePhase, ToolCallDisplay } from "../types";

const disposers: (() => void)[] = [];
afterEach(() => { disposers.splice(0).forEach(dispose => dispose()); });

function mount(native = true) {
  const props = reactive({
    messages: [{ id: "user", role: "user", content: "Hello", createdAt: 1 }] as ChatMessage[],
    streamingText: "",
    isStreaming: true,
    isCompacting: false,
    isThinking: false,
    claudeCodeMode: native,
    claudeCodePhase: null as ClaudeCodePhase | null,
    showThinkingBlocks: true,
    activeToolCalls: [] as ToolCallDisplay[],
    liveRenderParts: [] as AssistantRenderPart[],
    waitingLabel: "Standard waiting",
    compactingLabel: "Compacting",
    thinkingActiveLabel: "Standard thinking",
    sessionKey: "native-activity",
    variant: "session" as const,
  });
  const openThinking = vi.fn();
  const errors: unknown[] = [];
  const host = document.createElement("div");
  document.body.append(host);
  const app = createApp({ render: () => h(ChatTranscript, { ...props, onOpenThinking: openThinking }) });
  app.config.errorHandler = error => errors.push(error);
  app.use(createPinia());
  app.mount(host);
  disposers.push(() => { app.unmount(); host.remove(); expect(errors).toEqual([]); });
  const status = () => host.querySelector<HTMLElement>('[data-render-part-key="transient:claude-code"]');
  return { props, host, openThinking, status };
}

describe("Claude Code status-only transcript", () => {
  it("shows preparation, waiting, and thinking without a clickable thinking block or duplicate waiting row", async () => {
    const view = mount();
    for (const phase of ["starting", "recovering", "requesting", "thinking"] as const) {
      view.props.claudeCodePhase = phase;
      await nextTick();
      expect(view.status()?.textContent).toBe(t(`chat.transcript.claudeCode.${phase}`));
      expect(view.host.querySelectorAll('[data-render-part-kind="waiting"]')).toHaveLength(1);
      expect(view.status()?.querySelector('button, a, [role="button"], .is-clickable')).toBeNull();
      view.status()?.querySelector<HTMLElement>(".chat-transcript-thinking-header")?.click();
    }
    expect(view.openThinking).not.toHaveBeenCalled();
  });

  it("shows recovery during interrupted tools and removes it when the run ends", async () => {
    const view = mount();
    view.props.claudeCodePhase = "recovering";
    view.props.activeToolCalls = [{ id: "write", name: "edit", arguments: "{}", status: "running" }];
    await nextTick();
    expect(view.status()?.textContent).toBe(t("chat.transcript.claudeCode.recovering"));
    view.props.isStreaming = false;
    await nextTick();
    expect(view.status()).toBeNull();
    expect(view.host.textContent).not.toContain(t("chat.transcript.claudeCode.starting"));
  });

  it("suppresses both live and saved thinking summaries in native mode", async () => {
    const view = mount();
    const settings = useDisplaySettings().state;
    const previous = settings.hideThinkingBlocks;
    settings.hideThinkingBlocks = false;
    disposers.push(() => { settings.hideThinkingBlocks = previous; });
    view.props.claudeCodePhase = "thinking";
    view.props.isThinking = true;
    view.props.liveRenderParts = [{ kind: "thinking", id: "live-thought", order: { runId: "run", seq: 1 }, content: "Secret summary", active: true }];
    view.props.messages.push({ id: "saved", role: "assistant", content: "Saved answer", createdAt: 2, thinkingContent: "Saved summary", thinkingDuration: 3 });
    await nextTick();
    expect(view.host.querySelector('[data-render-part-kind="thinking"]')).toBeNull();
    expect(view.host.textContent).not.toContain("Secret summary");
    expect(view.host.textContent).not.toContain("Saved summary");
    expect(view.status()?.textContent).toBe(t("chat.transcript.claudeCode.thinking"));
    view.props.isStreaming = false;
    await nextTick();
    expect(view.status()).toBeNull();
    expect(view.openThinking).not.toHaveBeenCalled();
  });

  it.each(["tool-input", "tool-use"] as const)("keeps the previous status through %s until the tool block renders, then restores the next thinking status", async (phase) => {
    const view = mount();
    await nextTick();
    expect(view.status()?.textContent).toBe(t("chat.transcript.claudeCode.requesting"));
    view.props.claudeCodePhase = phase;
    await nextTick();
    // The phase has no label of its own; the previous status stays until output renders.
    expect(view.status()?.textContent).toBe(t("chat.transcript.claudeCode.requesting"));
    expect(view.host.querySelectorAll('[data-render-part-kind="waiting"]')).toHaveLength(1);
    view.props.activeToolCalls = [{ id: "read", name: "read", arguments: "{}", status: "running", order: 1 }];
    await nextTick();
    expect(view.host.querySelector('[data-render-part-kind="toolCall"]')).not.toBeNull();
    expect(view.status()).toBeNull();
    expect(view.host.querySelector('[data-render-part-kind="waiting"]')).toBeNull();
    view.props.claudeCodePhase = "thinking";
    await nextTick();
    expect(view.status()?.textContent).toBe(t("chat.transcript.claudeCode.thinking"));
    expect(view.host.querySelectorAll('[data-render-part-kind="waiting"]')).toHaveLength(1);
  });

  it("keeps the thinking status until response text renders, without falling back to waiting", async () => {
    const view = mount();
    view.props.claudeCodePhase = "thinking";
    await nextTick();
    expect(view.status()?.textContent).toBe(t("chat.transcript.claudeCode.thinking"));
    view.props.claudeCodePhase = "responding";
    await nextTick();
    // Thinking stays until the response text is visible.
    expect(view.status()?.textContent).toBe(t("chat.transcript.claudeCode.thinking"));
    expect(view.host.querySelectorAll('[data-render-part-kind="waiting"]')).toHaveLength(1);
    view.props.streamingText = "Answer";
    view.props.liveRenderParts = [{ kind: "text", id: "answer", order: { runId: "run", seq: 1 }, content: "Answer" }];
    await nextTick();
    expect(view.host.querySelector('[data-render-part-kind="text"]')?.textContent).toContain("Answer");
    expect(view.status()).toBeNull();
    expect(view.host.querySelector('[data-render-part-kind="waiting"]')).toBeNull();
    view.props.claudeCodePhase = "requesting";
    await nextTick();
    expect(view.status()?.textContent).toBe(t("chat.transcript.claudeCode.requesting"));
  });

  it.each(["tool-use", "responding"] as const)("prioritizes compaction over hidden %s status", async (phase) => {
    const view = mount();
    view.props.streamingText = "Working";
    view.props.liveRenderParts = [{ kind: "text", id: "text", order: { runId: "run", seq: 1 }, content: "Working" }];
    view.props.activeToolCalls = [{ id: "read", name: "read", arguments: "{}", status: "running", order: 2 }];
    view.props.claudeCodePhase = phase;
    await nextTick();
    expect(view.status()).toBeNull();
    expect(view.host.querySelector('[data-render-part-kind="waiting"]')).toBeNull();
    view.props.isCompacting = true;
    await nextTick();
    expect(view.status()?.textContent).toBe("Compacting");
    expect(view.host.querySelectorAll('[data-render-part-kind="waiting"]')).toHaveLength(1);
    view.props.isCompacting = false;
    await nextTick();
    expect(view.status()).toBeNull();
    view.props.claudeCodePhase = "responding";
    await nextTick();
    expect(view.status()).toBeNull();
    view.props.isStreaming = false;
    await nextTick();
    expect(view.status()).toBeNull();
  });

  it("falls back to waiting when the CLI cannot report its phase", async () => {
    const view = mount();
    await nextTick();
    expect(view.status()?.textContent).toBe(t("chat.transcript.claudeCode.requesting"));
  });

  it("leaves other providers' waiting and clickable thinking behavior unchanged", async () => {
    const view = mount(false);
    view.props.claudeCodePhase = "thinking"; // Native data must not opt in another provider.
    await nextTick();
    expect(view.status()).toBeNull();
    expect(view.host.textContent).toContain("Standard waiting");
    view.props.isThinking = true;
    view.props.liveRenderParts = [{ kind: "thinking", id: "thought", order: { runId: "run", seq: 1 }, content: "Normal summary", active: true }];
    await nextTick();
    expect(view.host.textContent).toContain("Standard thinking");
    view.host.querySelector<HTMLElement>('[data-render-part-kind="thinking"] .chat-transcript-thinking-header')?.click();
    expect(view.openThinking).toHaveBeenCalledOnce();
    expect(view.status()).toBeNull();
  });
});
