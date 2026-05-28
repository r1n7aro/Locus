import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("file diff popover interaction", () => {
  it("keeps the hover preview alive while the pointer is over the popover", () => {
    const popover = read("src/components/diff/FileDiffPopover.vue");
    const chatChanges = read("src/components/ChatChangesPanel.vue");
    const trigger = read("src/components/diff/FileDiffTrigger.vue");

    expect(popover).toContain("enter: [];");
    expect(popover).toContain("leave: [];");
    expect(popover).toContain("@mouseenter=\"emit('enter')\"");
    expect(popover).toContain("@mouseleave=\"emit('leave')\"");

    expect(chatChanges).toContain("function scheduleHoverClose()");
    expect(chatChanges).toContain("function onPopoverMouseEnter()");
    expect(chatChanges).toContain("@enter=\"onPopoverMouseEnter\"");
    expect(chatChanges).toContain("@leave=\"onPopoverMouseLeave\"");
    expect(chatChanges).toContain("if (!displaySettings.fileChangePopoverEnabled) return;");

    expect(trigger).toContain("function schedulePopoverClose()");
    expect(trigger).toContain("function onPopoverMouseEnter()");
    expect(trigger).toContain("@enter=\"onPopoverMouseEnter\"");
    expect(trigger).toContain("@leave=\"onPopoverMouseLeave\"");
    expect(trigger).toContain("if (!displaySettings.fileChangePopoverEnabled) return;");
  });

  it("exposes the full diff action from the popover footer", () => {
    const popover = read("src/components/diff/FileDiffPopover.vue");
    const chatChanges = read("src/components/ChatChangesPanel.vue");
    const trigger = read("src/components/diff/FileDiffTrigger.vue");

    expect(popover).toContain("open: [];");
    expect(popover).toContain("@click.stop=\"emit('open')\"");
    expect(chatChanges).toContain("@open=\"onPopoverOpen\"");
    expect(trigger).toContain("@open=\"onClick\"");
  });
});
