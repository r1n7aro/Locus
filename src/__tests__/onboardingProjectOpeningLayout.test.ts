import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("onboarding project opening feedback", () => {
  it("renders a busy state before switching workspace", () => {
    const source = read("src/components/OnboardingView.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(source).toContain("const projectOpening = ref(false);");
    expect(source).toContain("await waitForProjectOpeningPaint();");
    expect(source).toContain('projectOpening ? t("onboarding.project.opening") : t("onboarding.project.browse")');
    expect(source).toContain(':disabled="projectOpening"');
    expect(source).toContain(':aria-busy="projectOpening"');
    expect(source).toContain("project-opening-spinner");
    expect(zh).toContain('"onboarding.project.opening": "正在打开..."');
    expect(en).toContain('"onboarding.project.opening": "Opening..."');
  });
});
