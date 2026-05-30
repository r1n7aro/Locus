import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Session compact picker Views", () => {
  it("renders the session View tree when the session View setting is enabled", () => {
    const chatView = read("src/components/ChatView.vue");
    const compactPicker = read("src/components/chat/SessionCompactPicker.vue");

    expect(chatView).toContain(":show-views=\"displaySettings.showViewsInSessionPanel\"");
    expect(chatView).toContain(":working-dir=\"workingDir\"");

    expect(compactPicker).toContain("showViews?: boolean;");
    expect(compactPicker).toContain("workingDir?: string;");
    expect(compactPicker).toContain("const showSessionViews = computed(() => props.showViews !== false)");
    expect(compactPicker).toContain("viewTree()");
    expect(compactPicker).toContain("view-package-reloaded");
    expect(compactPicker).toContain("view-tree-changed");
    expect(compactPicker).toContain("class=\"session-compact-session-region\"");
    expect(compactPicker).toContain("class=\"session-compact-view-section\"");
    expect(compactPicker).toContain("class=\"session-compact-view-row\"");
    expect(compactPicker).toContain("flex: 0 0 142px;");
    expect(compactPicker).toContain(":icon=\"resolveLocusViewIcon(row.node.view?.icon)\"");
    expect(compactPicker).toContain("@click=\"onViewRowClick(row)\"");
  });
});
