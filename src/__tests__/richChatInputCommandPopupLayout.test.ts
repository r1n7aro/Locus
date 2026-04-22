import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("RichChatInput command popup layout", () => {
  it("renders the argument hint in the same header row as the command name", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");

    expect(richInput).toContain('<div class="command-header">');
    expect(richInput).toContain('<span v-if="command.argumentHint" class="command-hint-inline">{{ command.argumentHint }}</span>');
    expect(
      richInput.indexOf('<span v-if="command.argumentHint" class="command-hint-inline">{{ command.argumentHint }}</span>'),
    ).toBeLessThan(
      richInput.indexOf('<span class="command-kind-badge">{{ commandTypeLabel(command) }}</span>'),
    );
    expect(richInput).toContain('<span class="command-desc">{{ command.description }}</span>');
    expect(richInput).not.toContain('class="command-hint">{{ command.argumentHint }}</span>');
  });

  it("uses a stronger highlighted state than hover for the selected command", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");

    expect(richInput).toContain(".command-item:hover {");
    expect(richInput).toContain("background: var(--hover-bg);");
    expect(richInput).toContain(".command-item.highlighted {");
    expect(richInput).toContain("background: color-mix(in srgb, var(--accent-soft) 86%, var(--hover-bg) 14%);");
    expect(richInput).toContain("border-color: color-mix(in srgb, var(--accent-color) 28%, var(--border-color));");
    expect(richInput).toContain("box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 10%, transparent);");
  });

  it("keeps the highlighted command scrolled into view during keyboard navigation", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");

    expect(richInput).toContain("const commandPopupRef = ref<HTMLElement | null>(null);");
    expect(richInput).toContain("const commandItemRefs = ref<HTMLElement[]>([]);");
    expect(richInput).toContain("function setCommandItemRef(index: number, element: Element | ComponentPublicInstance | null) {");
    expect(richInput).toContain("() => [showCommandPopup.value, commandHighlightIndex.value, filteredCommands.value.length],");
    expect(richInput).toContain("const popup = commandPopupRef.value;");
    expect(richInput).toContain("const selected = commandItemRefs.value[commandHighlightIndex.value];");
    expect(richInput).toContain("popup.scrollTop = itemTop;");
    expect(richInput).toContain("popup.scrollTop = itemBottom - popup.clientHeight;");
    expect(richInput).toContain('ref="commandPopupRef"');
    expect(richInput).toContain(':ref="(el) => setCommandItemRef(index, el)"');
  });
});
