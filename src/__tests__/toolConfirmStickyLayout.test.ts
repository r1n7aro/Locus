import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("tool confirm sticky batch layout", () => {
  it("keeps the batch card active until the pending set is cleared", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toContain("const keepBatchToolConfirmLayout = ref(false)");
    expect(chatView).toContain("const showBatchToolConfirmCard = computed(() =>");
    expect(chatView).toContain("const showSingleToolConfirmCard = computed(() =>");
    expect(chatView).toContain("keepBatchToolConfirmLayout.value = count > 1");
    expect(chatView).toContain("keepBatchToolConfirmLayout.value = true");
    expect(chatView).toContain("keepBatchToolConfirmLayout.value = false");
    expect(chatView).toContain('v-if="showBatchToolConfirmCard"');
    expect(chatView).toContain('v-else-if="showSingleToolConfirmCard"');
  });
});
