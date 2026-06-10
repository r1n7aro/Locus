import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

function readJson<T>(relPath: string): T {
  return JSON.parse(read(relPath)) as T;
}

describe("sheet confirmation card", () => {
  it("submits confirm and feedback answers as structured JSON", () => {
    const card = read("src/components/chat/SheetCard.vue");

    expect(card).toContain('JSON.stringify({ action: "confirm", values: submitted, feedback: feedback.value.trim() })');
    expect(card).toContain('JSON.stringify({ action: "feedback", feedback: feedback.value.trim() })');
    // readonly fields are display-only and never submitted
    expect(card).toContain("if (!field.readonly) submitted[field.key]");
    expect(card).toContain('class="sheet-field-readonly"');
    // a revised sheet reuses the card, so state resets per question
    expect(card).toContain("() => props.question.questionId");
    expect(card).toContain('feedback.value = ""');
  });

  it("renders field controls by shape and keeps the current value selectable", () => {
    const card = read("src/components/chat/SheetCard.vue");

    expect(card).toContain("sheet-field-select");
    expect(card).toContain("sheet-field-textarea");
    expect(card).toContain("sheet-field-input");
    expect(card).toContain("options.includes(current) ? options : [current, ...options]");
    expect(card).toContain('t("chat.sheet.requestChanges")');
    expect(card).toContain(":disabled=\"!hasFeedback\"");
    expect(card).toContain('props.question.sheet?.confirmLabel || t("chat.sheet.confirm")');
    // reuses the ask-user-card shell so both chat containers position it correctly
    expect(card).toContain('class="ask-user-card sheet-card"');
  });

  it("is rendered for sheet questions in both chat containers with ask fallback", () => {
    const chatView = read("src/components/ChatView.vue");
    const embeddedPane = read("src/components/chat/EmbeddedChatPane.vue");

    expect(chatView).toContain('import SheetCard from "./chat/SheetCard.vue"');
    expect(chatView).toContain('v-if="pendingQuestion && pendingQuestion.sheet && !isViewingSubagent"');
    expect(chatView).toContain('v-else-if="pendingQuestion && !isViewingSubagent"');

    expect(embeddedPane).toContain('import SheetCard from "./SheetCard.vue"');
    expect(embeddedPane).toContain('v-if="pendingQuestion && pendingQuestion.sheet"');
    expect(embeddedPane).toContain('v-else-if="pendingQuestion"');
  });

  it("keeps the sheet payload in shared chat types", () => {
    const types = read("src/types.ts");

    expect(types).toContain("export interface SheetField {");
    expect(types).toContain("export interface SheetRequest {");
    expect(types).toMatch(/interface PendingQuestion \{[^}]*sheet\?: SheetRequest \| null;/s);
  });

  it("declares sheet strings and permission row", () => {
    const en = readJson<Record<string, string>>("src/language/en.json");
    const zh = readJson<Record<string, string>>("src/language/zh.json");
    const settings = read("src/composables/useSettingsState.ts");

    for (const key of [
      "chat.sheet.confirm",
      "chat.sheet.requestChanges",
      "chat.sheet.feedbackPlaceholder",
      "chat.sheet.modified",
      "tool.desc.sheet",
    ]) {
      expect(en[key], `en ${key}`).toBeTruthy();
      expect(zh[key], `zh ${key}`).toBeTruthy();
    }

    expect(settings).toContain('{ name: "sheet",');
  });
});
