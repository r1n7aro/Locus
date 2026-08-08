import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("onboarding custom provider layout", () => {
  it("opens the shared provider modal instead of expanding the setup form inline", () => {
    const source = read("src/components/OnboardingView.vue");

    expect(source).toContain('import CustomProviderModal from "./settings/CustomProviderModal.vue"');
    expect(source).toContain("openCustomProviderConfiguration");
    expect(source).toContain("toggleCodexProvider");
    expect(source).toContain("settingsEditingProvider");
    expect(source).toContain("settingsCustomProviders");
    expect(source).toContain('v-model:provider="settingsEditingProvider"');
    expect(source).toContain(':is-adding="settingsIsAddingProvider"');
    expect(source).toContain('@save="settingsSaveProvider"');
    expect(source).toContain('@test="settingsTestProvider"');
    expect(source).toContain('@open-catalog="settingsLoadModelCatalog()"');
    expect(source).toContain("settingsStartEditProvider(provider)");
    expect(source).toContain("settingsStartAddProvider()");
    expect(source).toContain("max-width: 600px;");
    expect(source).toContain('<button\n            class="provider-header"');
    expect(source).not.toContain("authExpanded === 'custom'");
    expect(source).not.toContain("custom-endpoint-fields");
    expect(source).not.toContain("custom-endpoint-test-result");
    expect(source).not.toContain("ModelCatalogPicker");
    expect(source).not.toContain("BaseDropdown");
    expect(source).not.toContain("<select");
    expect(source).not.toContain("ApiProviders");
  });
});
