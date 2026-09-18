import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("knowledge selection composer attachments", () => {
  it("adds selected knowledge as an attachment instead of composer text", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const quoteStart = workbench.indexOf("async function quoteKnowledgeSelectionInConversation");
    const quoteEnd = workbench.indexOf("async function openFileInWorkbench", quoteStart);
    const quoteFlow = workbench.slice(quoteStart, quoteEnd);

    expect(quoteFlow).toContain("draft.knowledgeQuotes = [quote]");
    expect(quoteFlow).toContain("await session.appendComposerDraft(draft)");
    expect(quoteFlow).not.toContain("draft.text =");
    expect(quoteFlow).not.toContain("session.applyDraftPrefill");
  });

  it("renders, removes, persists, and sends the quote attachment", () => {
    const input = read("src/components/chat/RichChatInput.vue");

    expect(input).toContain('v-for="(quote, index) in knowledgeQuoteAttachments"');
    expect(input).toContain('class="local-file-chip knowledge-quote-chip"');
    expect(input).toContain('@click.stop="removeKnowledgeQuoteAttachment(index)"');
    expect(input).toContain("knowledgeQuotes: knowledgeQuoteAttachments.value.map");
    expect(input).toContain("addKnowledgeQuoteAttachments(draft.knowledgeQuotes ?? [])");
    expect(input).toContain("appendKnowledgeQuotePromptBlock(text, quotes)");
    expect(input).toContain("...knowledgeQuoteAssetRefs(quotes)");
  });

  it("uses concise localized attachment labels", () => {
    expect(read("src/language/zh.json")).toContain('"chat.knowledgeQuote.selection": "知识选区"');
    expect(read("src/language/en.json")).toContain('"chat.knowledgeQuote.selection": "Knowledge selection"');
  });
});
