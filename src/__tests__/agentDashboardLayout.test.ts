import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("AgentView dashboard layout", () => {
  it("shows the prompt dashboard in the no-selection preview state", () => {
    const source = read("src/components/AgentView.vue");

    expect(source).toContain("getAgentSystemPromptStats");
    expect(source).toContain("buildAgentPromptDashboard");
    expect(source).toContain("v-else class=\"preview-panel dashboard-panel\"");
    expect(source).toContain("agent.dashboard.title");
    expect(source).toContain("dashboard-top-grid");
    expect(source).toContain("dashboard-bottom-grid");
    expect(source).toContain("dashboard-card-breakdown");
    expect(source).toContain("dashboard-card-runtime");
  });
});
