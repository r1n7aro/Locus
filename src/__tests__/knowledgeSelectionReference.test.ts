import { EditorSelection, EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { captureMarkdownEditorSelection } from "../components/ui/markdown-editor/markdownEditorSelection";
import { knowledgeSelectionReference } from "../components/knowledge/knowledgeSelectionReference";
import { parseInlineIntentCommands } from "../composables/chatInputIntents";

const labels = { draft: "unsaved draft", unavailable: "source changed" };
function selection(doc: string, from = 0, to = doc.length) {
  return captureMarkdownEditorSelection(EditorState.create({ doc, selection: { anchor: to, head: from } }));
}
const source = (content: string) => ({ path: "F:\\Project\\Locus\\knowledge\\design\\boss.md", content });

describe("knowledge selection references", () => {
  it("uses physical source lines with CRLF frontmatter and repeated selected words", () => {
    const body = "# Boss\nrepeat\nother\nrepeat\nend";
    const raw = "---\nid: boss\nsummary: repeat\n---\n\n" + body;
    const from = body.lastIndexOf("repeat");
    const result = knowledgeSelectionReference(source(raw.replace(/\n/g, "\r\n")), "body", body, selection(body, from, from + 7), labels);
    expect(result).toContain("F:/Project/Locus/knowledge/design/boss.md:9\n");
    expect(result).toContain("repeat\n");
    expect(result).not.toContain("unsaved");
  });

  it("keeps the snapshot and identifies unsaved draft line numbers", () => {
    const saved = "# Boss\noriginal";
    const current = "# Boss\nnew line\nlocal draft";
    const result = knowledgeSelectionReference(source("---\nid: boss\n---\n\n" + saved), "body", saved, selection(current, current.indexOf("local")), labels);
    expect(result).toContain("boss.md:7 (unsaved draft)");
    expect(result).toContain("local draft");
  });

  it("quotes YAML scalar fields by their actual source range", () => {
    const raw = "---\nid: boss\nsummary: |-\n  first\n  second\nmaintenanceRules: \"a\\nb\"\n---\n\nbody";
    const summary = knowledgeSelectionReference(source(raw), "summary", "first\nsecond", selection("first\nsecond", 6), labels);
    expect(summary).toContain("boss.md:3-5\n");
    const rules = knowledgeSelectionReference(source(raw), "maintenanceRules", "a\nb", selection("a\nb"), labels);
    expect(rules).toContain("boss.md:6\n");
  });

  it("accounts for trimmed leading blank lines and handles multiple selections independently", () => {
    const doc = "\n\none\ntwo\nthree\n";
    const state = EditorState.create({
      doc,
      extensions: [EditorState.allowMultipleSelections.of(true)],
      selection: EditorSelection.create([EditorSelection.range(2, 5), EditorSelection.range(10, 15)]),
    });
    const result = knowledgeSelectionReference(source("---\nid: a\n---\n\none\ntwo\nthree\n"), "body", "one\ntwo\nthree", captureMarkdownEditorSelection(state), labels);
    expect(result).toContain("boss.md:5\n");
    expect(result).toContain("boss.md:7\n");
  });

  it("does not let source code fences break out of the quote", () => {
    const body = "```js\nconst a = 1;\n```";
    expect(knowledgeSelectionReference(source(body), "body", body, selection(body), labels)).toContain("````markdown\n```js");
  });

  it("rejects stale or ambiguous sources instead of guessing line numbers", () => {
    expect(() => knowledgeSelectionReference(source("different"), "body", "old", selection("old"), labels)).toThrow("source changed");
  });

  it("keeps slash commands inside a quote as document content when sending to AI", () => {
    const body = "/plan\n```\n    /plan\n\n\n    indented code\n```";
    const quote = knowledgeSelectionReference(source(body), "body", body, selection(body), labels);
    const commands = [{ name: "/plan", description: "", commandKind: "intent" as const, commandType: "plan" as const }];
    const parsed = parseInlineIntentCommands(quote, commands, "agent");
    expect(parsed.intent.mode).toBe("build");
    expect(parsed.cleanedText).toBe(quote);
    const withIntent = parseInlineIntentCommands(`${quote}\n\n/plan`, commands, "agent");
    expect(withIntent.intent.mode).toBe("plan");
    expect(withIntent.cleanedText).toBe(quote);
  });
});
