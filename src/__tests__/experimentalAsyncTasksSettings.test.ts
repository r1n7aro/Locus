import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");

describe("experimental async task settings", () => {
  it("exposes a dedicated settings page with the shared switch", () => {
    const settingsView = readFileSync(resolve(root, "components/SettingsView.vue"), "utf8");
    const experimental = readFileSync(
      resolve(root, "components/settings/ExperimentalSettings.vue"),
      "utf8",
    );

    expect(settingsView).toContain("activeCategory === 'experimental'");
    expect(settingsView).toContain("<ExperimentalSettings />");
    expect(experimental).toContain("BaseSwitch");
    expect(experimental).toContain("getAsyncTasksEnabled");
    expect(experimental).toContain("setAsyncTasksEnabled");
  });
});
