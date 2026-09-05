import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Unity agent work-in-progress output rules", () => {
  it("keeps the progress-update contract enabled in the runtime prompt", () => {
    const rules = read("agent/unity/rule/output_principles.md");
    const config = JSON.parse(read("agent/unity/rule_config.json"));

    expect(config["output_principles.md"]).toMatchObject({ enabled: true });
    expect(rules).toContain("For multi-step work");
    expect(rules).toContain("before substantial tool use");
    expect(rules).toContain("meaningful milestones");
    expect(rules).toContain("group related calls");
    expect(rules).toContain("final answer must stand on its own");
  });
});
