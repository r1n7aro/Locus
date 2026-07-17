import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function read(relPath: string) {
  return readFileSync(resolve(process.cwd(), relPath), "utf8");
}

describe("experimental features settings", () => {
  it("uses the persisted Claude Code model flag as the single feature switch", () => {
    const settings = read("src/components/SettingsView.vue");
    const experimental = read("src/components/settings/ExperimentalFeaturesSettings.vue");
    const providers = read("src/components/settings/ApiProviders.vue");
    const defaults = read("src/components/settings/ModelDefaults.vue");
    const modelStore = read("src/stores/model.ts");

    expect(settings).toContain("ExperimentalFeaturesSettings");
    expect(settings).toContain("setClaudeCodeEnabled");
    expect(experimental).toContain("BaseSwitch");
    expect(experimental).toContain("emit('update:claudeCodeEnabled', $event)");
    expect(providers).toContain("claudeCodeEnabled && claudeCodeProvider");
    expect(defaults).not.toContain("updateClaudeCodeEnabled");
    expect(modelStore).toContain("modelDefaults.value.claudeCodeEnabled === true");
  });
});
