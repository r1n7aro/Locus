import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { buildCodexQuotaSummary } from "../services/codexQuotaSummary";
import type { CodexRateLimitsResponse } from "../services/auth";

const cwd = process.cwd();

function read(relativePath: string): string {
  return readFileSync(resolve(cwd, relativePath), "utf8");
}

describe("session context usage", () => {
  it("normalizes the Codex primary and secondary remaining quota", () => {
    const response: CodexRateLimitsResponse = {
      fetchedAtMs: 1,
      rateLimits: {
        limitId: "codex",
        primary: {
          usedPercent: 28.4,
          remainingPercent: 71.6,
          windowMinutes: 300,
        },
        secondary: {
          usedPercent: 110,
          remainingPercent: -10,
          windowMinutes: 10080,
        },
      },
      rateLimitsByLimitId: {},
    };

    expect(buildCodexQuotaSummary(response)).toEqual([
      {
        key: "primary",
        limitId: "codex",
        limitName: null,
        remainingPercent: 71.6,
        windowMinutes: 300,
      },
      {
        key: "secondary",
        limitId: "codex",
        limitName: null,
        remainingPercent: 0,
        windowMinutes: 10080,
      },
    ]);
  });

  it("opens a restrained in-app statistics overlay from the context ring", () => {
    const bar = read("src/components/chat/TokenUsageBar.vue");
    const chat = read("src/components/ChatView.vue");
    const router = read("src/WindowApp.vue");
    const window = read("src/components/SessionContextUsageWindow.vue");

    expect(bar).toContain('emit("openContextStats")');
    expect(bar).toContain('cursor: default');
    expect(bar).toContain('class="context-usage-line context-quota-line"');
    expect(chat).toContain(':active-session-id="activeSessionId"');
    expect(chat).toContain('@open-context-stats="contextStatsOpen = true"');
    expect(chat).toContain('<SessionContextUsageWindow');
    expect(router).not.toContain('kind: "session-context-usage"');
    expect(window).toContain('class="context-stats-backdrop"');
    expect(window).toContain('role="dialog"');
    expect(window).toContain('aria-modal="true"');
    expect(window).toContain('class="context-token-metrics"');
    expect(window).toContain('t("chat.contextStats.outputSpeed")');
    expect(window).toContain("calculateAverageOutputTokensPerSecond(usage)");
    expect(window).toContain('t("chat.contextStats.outputSpeedHelp")');
    expect(window).toContain('class="context-breakdown-list"');
    expect(window).toContain('class="context-tools-table"');
    expect(window).toContain("tool.resultTokens");
    expect(window).toContain("tool.callCount");
    expect(window).not.toContain("shareLabel");
    expect(window).not.toContain("openSubWindow");
    expect(window).not.toMatch(/#[0-9a-fA-F]{3,8}/);
  });

  it("reports provider usage and groups tool result consumption by tool", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const commands = read("src-tauri/src/commands/session.rs");
    const commandTypes = read("src-tauri/src/commands/mod.rs");
    const lib = read("src-tauri/src/lib.rs");

    expect(agent).toContain("pub async fn session_context_usage_report(");
    expect(agent).toContain("estimate_session_tool_result_usage");
    expect(agent).toContain("message.role == MessageRole::Tool");
    expect(agent).toContain("estimate_api_tool_prompt_tokens");
    expect(agent).toContain("environment_tokens");
    expect(agent).toContain("runtime_injection_tokens");
    expect(agent).toContain("active_tool_result_tokens");
    expect(agent).toContain("conversation_tokens");
    expect(agent).toContain("response_model_active_duration_ms");
    expect(agent).toContain("let llm_call_started_at = Instant::now()");
    expect(commands).toContain("pub async fn get_session_context_usage_report(");
    expect(commands).toContain("store.get_messages(&session_id)");
    expect(commandTypes).toContain("pub struct SessionContextUsageReport");
    expect(commandTypes).toContain("pub result_tokens: u32");
    expect(commandTypes).toContain("pub call_count: u32");
    expect(commandTypes).toContain("pub usage: TokenUsage");
    expect(commandTypes).toContain("pub timed_output_tokens: u64");
    expect(commandTypes).toContain("pub model_active_duration_ms: u64");
    expect(lib).toContain("commands::get_session_context_usage_report");
  });
});
