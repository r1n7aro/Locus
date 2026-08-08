// @vitest-environment jsdom
import { createApp, nextTick } from "vue";
import { describe, expect, it } from "vitest";
import ToolSearchOutput from "../components/ToolSearchOutput.vue";
import { buildToolCallArgsSummary } from "../components/toolCallSummary";
import {
  parseToolSearchOutput,
  summarizeToolSearchDescription,
} from "../components/toolSearchOutput";

const SEARCH_OUTPUT = JSON.stringify({
  tools: [
    {
      type: "function",
      name: "unity_run_states",
      description: [
        "Run a structured C# state machine inside the connected Unity Editor.",
        "",
        "Use this tool when the Agent needs to observe Unity across frames.",
      ].join("\n"),
      parameters: {
        type: "object",
        properties: {
          initial_state: { type: "string" },
          states: { type: "array" },
        },
      },
      defer_loading: true,
    },
    {
      type: "function",
      name: "web_fetch",
      description: "Fetch content from a URL.",
      parameters: { type: "object" },
      defer_loading: true,
    },
  ],
});

describe("tool_search output formatting", () => {
  it("uses the exact wire_names as the call summary", () => {
    expect(buildToolCallArgsSummary("tool_search", JSON.stringify({
      wire_names: ["unity_run_states", "unity_capture_viewport"],
    }))).toBe("unity_run_states, unity_capture_viewport");
  });

  it("extracts exact tool names and compact first-paragraph descriptions", () => {
    expect(parseToolSearchOutput(SEARCH_OUTPUT)).toEqual({
      tools: [
        {
          name: "unity_run_states",
          description: "Run a structured C# state machine inside the connected Unity Editor.",
        },
        {
          name: "web_fetch",
          description: "Fetch content from a URL.",
        },
      ],
    });
  });

  it("accepts an empty result and rejects unrelated or malformed output", () => {
    expect(parseToolSearchOutput('{"tools":[]}')).toEqual({ tools: [] });
    expect(parseToolSearchOutput('{"result":[]}')).toBeNull();
    expect(parseToolSearchOutput("not json")).toBeNull();
  });

  it("bounds unusually long single-paragraph descriptions", () => {
    const summary = summarizeToolSearchDescription("x".repeat(400));
    expect(summary).toHaveLength(240);
    expect(summary.endsWith("…")).toBe(true);
  });

  it("renders a readable tool list without the parameter schema", async () => {
    const parsed = parseToolSearchOutput(SEARCH_OUTPUT);
    expect(parsed).not.toBeNull();

    const host = document.createElement("div");
    const app = createApp(ToolSearchOutput, { tools: parsed!.tools });
    app.mount(host);
    await nextTick();

    expect(host.querySelectorAll(".tool-search-item")).toHaveLength(2);
    expect(host.querySelectorAll(".tool-search-name")[0]?.textContent).toBe("unity_run_states");
    expect(host.textContent).toContain("web_fetch");
    expect(host.textContent).toContain("Run a structured C# state machine");
    expect(host.textContent).not.toContain("initial_state");
    expect(host.textContent).not.toContain("Use this tool when");

    app.unmount();
  });
});
