import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Dev agent work-in-progress output rules", () => {
  it("keeps the progress-update contract enabled in the runtime prompt", () => {
    const rules = read("agent/dev/rule/output_principles.md");
    const config = JSON.parse(read("agent/dev/rule_config.json"));

    expect(config["output_principles.md"]).toMatchObject({ enabled: true });
    expect(rules).toContain("Work-in-progress updates:");
    expect(rules).toContain("before the first substantial tool-call batch");
    expect(rules).toContain("when a meaningful phase finishes and more work remains");
    expect(rules).toContain("more than four consecutive tool-call rounds");
    expect(rules).toContain("Group related actions into one update");
    expect(rules).toContain("Skip progress updates for a direct answer or an isolated trivial tool call");
  });
});
