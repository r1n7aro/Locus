import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { parseUnityPropertyFence } from "../composables/unityPropertyFence";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Unity property display rule", () => {
  it("keeps Unity property display independently configurable", () => {
    const referenceRule = read("agent/unity/rule/unity_reference_protocol.md");
    const propertyRule = read("agent/unity/rule/unity_property_display.md");
    const config = JSON.parse(read("agent/unity/rule_config.json"));

    expect(config["unity_reference_protocol.md"]).toMatchObject({
      enabled: true,
    });
    expect(config["unity_property_display.md"]).toMatchObject({
      enabled: true,
    });
    expect(config["unity_property_display.md"].order).toBeGreaterThan(
      config["unity_reference_protocol.md"].order,
    );
    expect(referenceRule).not.toContain("unity_property");
    const example = propertyRule.match(/```unity_property\r?\n([\s\S]*?)```/);
    expect(example).not.toBeNull();
    const parsed = parseUnityPropertyFence(example![1]);
    expect(parsed.issues).toEqual([]);
    expect(parsed.entries.map((entry) => entry.target.propertyPath)).toEqual([
      "damage",
      "m_IsActive",
      "m_LocalPosition",
    ]);
  });
});
