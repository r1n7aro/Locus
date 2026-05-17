import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("RichChatInput mention popup state", () => {
  it("keeps mention empty state tied to the settled query", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");
    const mentionPopup = read("src/components/chat/MentionPopup.vue");

    expect(richInput).toContain('const mentionSearchSettledQuery = ref("");');
    expect(richInput).toContain("const mentionPopupLoading = computed(() => {");
    expect(richInput).toContain("return mentionSearchSettledQuery.value !== mentionQuery.value;");
    expect(richInput).toContain("return mentionEntriesPath.value !== mentionSubPath.value;");
    expect(richInput).toContain("const showMentionEmpty = computed(() =>");
    expect(richInput).toContain("!mentionPopupLoading.value && mentionDisplayList.value.length === 0");
    expect(richInput).toContain(":loading=\"mentionPopupLoading\"");
    expect(richInput).toContain(":show-empty=\"showMentionEmpty\"");

    expect(mentionPopup).toContain("showEmpty: boolean;");
    expect(mentionPopup).toContain('v-else-if="showEmpty"');
    expect(mentionPopup).not.toContain('v-else-if="entries.length === 0"');
  });

  it("renders incremental loading in the mention header", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");
    const mentionPopup = read("src/components/chat/MentionPopup.vue");

    expect(mentionPopup).toContain('class="mention-loading-status"');
    expect(mentionPopup).toContain('loading && entries.length > 0');
    expect(mentionPopup).not.toContain("mention-loading-inline");
    expect(richInput).toContain(":deep(.mention-loading-status)");
    expect(richInput).not.toContain(":deep(.mention-loading-inline)");
  });
});
