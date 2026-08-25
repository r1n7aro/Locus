import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Unity property display rule", () => {
  it("keeps Unity property display independently configurable", () => {
    const referenceRule = read("agent/dev/rule/unity_reference_protocol.md");
    const propertyRule = read("agent/dev/rule/unity_property_display.md");
    const config = JSON.parse(read("agent/dev/rule_config.json"));

    expect(config["unity_reference_protocol.md"]).toMatchObject({
      enabled: true,
      order: 12,
    });
    expect(config["unity_property_display.md"]).toMatchObject({
      enabled: true,
      order: 13,
    });
    expect(referenceRule).not.toContain("unity_property");
    expect(propertyRule).toContain("fenced `unity_property` blocks");
    expect(propertyRule).toContain("```unity_property");
  });
});
