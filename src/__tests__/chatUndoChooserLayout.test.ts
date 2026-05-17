import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat undo chooser", () => {
  it("defaults to file undo when available and supports keyboard selection", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toContain('type UndoChoice = "conversation" | "files";');
    expect(chatView).toContain('const selectedUndoChoice = ref<UndoChoice>("conversation");');
    expect(chatView).toContain('return canUndoFilesAndConversation.value ? "files" : "conversation";');
    expect(chatView).toContain("selectedUndoChoice.value = defaultUndoChoice();");
    expect(chatView).toContain('ref="undoChooserRef"');
    expect(chatView).toContain('tabindex="-1"');
    expect(chatView).toContain('@keydown="handleUndoChooserKeydown"');
    expect(chatView).toContain('if (event.key === "ArrowDown") {');
    expect(chatView).toContain("moveUndoChoice(1);");
    expect(chatView).toContain('if (event.key === "ArrowUp") {');
    expect(chatView).toContain("moveUndoChoice(-1);");
    expect(chatView).toContain('if (event.key === "Enter") {');
    expect(chatView).toContain("runSelectedUndoChoice();");
    expect(chatView).toContain(':class="{ \'is-selected\': selectedUndoChoice === \'files\' }"');
    expect(chatView).toContain(".undo-chooser-action.is-selected:not(:disabled)");
  });
});
