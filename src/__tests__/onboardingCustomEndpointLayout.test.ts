import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("onboarding custom endpoint layout", () => {
  it("keeps model setup inline and exposes custom endpoint testing", () => {
    const source = read("src/components/OnboardingView.vue");

    expect(source).toContain("toggleAuthProvider('custom')");
    expect(source).toContain("toggleAuthProvider('codex')");
    expect(source).toContain('import BaseDropdown from "./ui/BaseDropdown.vue"');
    expect(source).toContain("settingsEditingEndpoint");
    expect(source).toContain('class="custom-endpoint-format-dropdown"');
    expect(source).toContain("@click=\"settingsTestEndpoint\"");
    expect(source).toContain('t("settings.custom.test")');
    expect(source).toContain('class="custom-endpoint-test-result"');
    expect(source).toContain("max-width: 600px;");
    expect(source).toContain("grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);");
    expect(source).toContain(".custom-endpoint-field:nth-child(5)");
    expect(source).toContain("box-sizing: border-box;");
    expect(source).toContain("min-width: 0;");
    expect(source).toContain("flex-shrink: 0;");
    expect(source).toContain("white-space: nowrap;");
    expect(source).not.toContain("<select");
    expect(source).not.toContain("ob-select");
    expect(source).not.toContain("model-config-overlay");
    expect(source).not.toContain("CustomEndpointModal");
    expect(source).not.toContain("ApiProviders");
  });
});
