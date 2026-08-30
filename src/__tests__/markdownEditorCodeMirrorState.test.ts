import { history, undo } from "@codemirror/commands";
import { EditorState, Transaction } from "@codemirror/state";
import { syntaxTree } from "@codemirror/language";
import { classHighlighter, highlightTree } from "@lezer/highlight";
import type { EditorView } from "@codemirror/view";
import { describe, expect, it } from "vitest";
import {
  DEFAULT_MARKDOWN_EDITOR_SESSION_LIMIT,
  MarkdownEditorSessionCache,
} from "../components/ui/markdown-editor/markdownEditorSessionCache";
import { createMinimalTextChange } from "../components/ui/markdown-editor/markdownEditorTransactions";
import {
  markdownEditorLanguageExtension,
  markdownEditorLanguageFromPath,
} from "../components/ui/markdown-editor/codeMirrorMarkdownExtensions";

describe("CodeMirror markdown editor state", () => {
  it("creates granular external changes without discarding equal spans", () => {
    const current = "alpha beta gamma";
    const next = "alpha stable gamma";
    const change = createMinimalTextChange(current, next);
    expect(change).toEqual([
      { from: 6, to: 8, insert: "s" },
      { from: 10, to: 10, insert: "ble" },
    ]);

    const state = EditorState.create({ doc: current });
    expect(state.update({ changes: change! }).state.doc.toString()).toBe(next);
    expect(createMinimalTextChange(next, next)).toBeNull();
  });

  it("handles insertions, deletions, and normalized unicode text", () => {
    expect(createMinimalTextChange("ac", "abc")).toEqual({ from: 1, to: 1, insert: "b" });
    expect(createMinimalTextChange("abc", "ac")).toEqual({ from: 1, to: 2, insert: "" });
    expect(createMinimalTextChange("标题：旧值", "标题：新值")).toEqual({
      from: 3,
      to: 4,
      insert: "新",
    });
  });

  it("preserves local undo history across disjoint external edits", () => {
    let state = EditorState.create({
      doc: "aaa middle zzz",
      extensions: [history()],
    });
    state = state.update({
      changes: { from: 4, to: 10, insert: "MIDDLE!" },
    }).state;

    const externalChange = createMinimalTextChange(
      state.doc.toString(),
      "AAA MIDDLE! ZZZ",
    );
    expect(externalChange).toEqual([
      { from: 0, to: 3, insert: "AAA" },
      { from: 12, to: 15, insert: "ZZZ" },
    ]);
    state = state.update({
      changes: externalChange!,
      annotations: Transaction.addToHistory.of(false),
    }).state;

    const undoTarget = {
      get state() {
        return state;
      },
      dispatch(transaction: Parameters<EditorView["dispatch"]>[0]) {
        state = state.update(transaction).state;
      },
    } as unknown as EditorView;
    expect(undo(undoTarget)).toBe(true);
    expect(state.doc.toString()).toBe("AAA middle ZZZ");
  });

  it("preserves a one-character local history anchor between nearby external edits", () => {
    let state = EditorState.create({
      doc: "a old b",
      extensions: [history()],
    });
    state = state.update({
      changes: { from: 2, to: 5, insert: "X" },
    }).state;

    const externalChange = createMinimalTextChange(state.doc.toString(), "c X d");
    expect(externalChange).toEqual([
      { from: 0, to: 1, insert: "c" },
      { from: 4, to: 5, insert: "d" },
    ]);
    state = state.update({
      changes: externalChange!,
      annotations: Transaction.addToHistory.of(false),
    }).state;

    const undoTarget = {
      get state() {
        return state;
      },
      dispatch(transaction: Parameters<EditorView["dispatch"]>[0]) {
        state = state.update(transaction).state;
      },
    } as unknown as EditorView;
    expect(undo(undoTarget)).toBe(true);
    expect(state.doc.toString()).toBe("c old d");
  });

  it("keeps repetitive 1 MB multi-point external diffs within a bounded hot path", () => {
    const current = `${"a".repeat(524_288)}MIDDLE${"z".repeat(524_281)}`;
    const next = `AAA${current.slice(3, -3)}ZZZ`;
    const startedAt = performance.now();
    const change = createMinimalTextChange(current, next);
    const elapsed = performance.now() - startedAt;

    expect(change).toEqual([
      { from: 0, to: 3, insert: "AAA" },
      { from: current.length - 3, to: current.length, insert: "ZZZ" },
    ]);
    expect(elapsed).toBeLessThan(250);
    const state = EditorState.create({ doc: current });
    expect(state.update({ changes: change! }).state.doc.toString()).toBe(next);
  });

  it("maps local undo through large unequal-length external anchors", () => {
    const prefix = "a".repeat(100_000);
    const suffix = "z".repeat(100_000);
    let state = EditorState.create({
      doc: `${prefix}middle${suffix}`,
      extensions: [history()],
    });
    state = state.update({
      changes: {
        from: prefix.length,
        to: prefix.length + "middle".length,
        insert: "MIDDLE!",
      },
    }).state;
    const external = `START${state.doc.toString()}END`;
    const change = createMinimalTextChange(state.doc.toString(), external);
    state = state.update({
      changes: change!,
      annotations: Transaction.addToHistory.of(false),
    }).state;

    const undoTarget = {
      get state() {
        return state;
      },
      dispatch(transaction: Parameters<EditorView["dispatch"]>[0]) {
        state = state.update(transaction).state;
      },
    } as unknown as EditorView;
    expect(undo(undoTarget)).toBe(true);
    expect(state.doc.toString()).toBe(`START${prefix}middle${suffix}END`);
  });

  it("keeps the most recently used editor states within the LRU bound", () => {
    const cache = new MarkdownEditorSessionCache(2);
    const snapshot = (doc: string) => ({
      state: EditorState.create({ doc }),
      scrollTop: doc.length,
      scrollLeft: 0,
    });

    cache.set("a", snapshot("a"));
    cache.set("b", snapshot("bb"));
    expect(cache.get("a")?.state.doc.toString()).toBe("a");
    cache.set("c", snapshot("ccc"));

    expect(cache.keys()).toEqual(["a", "c"]);
    expect(cache.has("b")).toBe(false);
    expect(DEFAULT_MARKDOWN_EDITOR_SESSION_LIMIT).toBe(12);
  });

  it("keeps pinned undo states beyond the soft limit until they become clean", () => {
    const cache = new MarkdownEditorSessionCache(2);
    const snapshot = (doc: string, pinned: boolean) => ({
      state: EditorState.create({ doc, extensions: [history()] }),
      scrollTop: 0,
      scrollLeft: 0,
      pinned,
    });

    cache.set("dirty-a", snapshot("a", true));
    cache.set("conflicted-b", snapshot("b", true));
    cache.set("blocked-c", snapshot("c", true));
    expect(cache.size).toBe(3);
    expect(cache.keys()).toEqual(["dirty-a", "conflicted-b", "blocked-c"]);

    cache.setPinned("dirty-a", false);
    expect(cache.size).toBe(2);
    expect(cache.has("dirty-a")).toBe(false);
    expect(cache.has("conflicted-b")).toBe(true);
    expect(cache.has("blocked-c")).toBe(true);
  });

  it("maps Markdown, JSON, Python, and C# paths to explicit language compartments", () => {
    expect(markdownEditorLanguageFromPath("notes/design.md")).toBe("markdown");
    expect(markdownEditorLanguageFromPath("config/settings.JSON?rev=2")).toBe("json");
    expect(markdownEditorLanguageFromPath("scripts/build.py#L12")).toBe("python");
    expect(markdownEditorLanguageFromPath("src/player.cs")).toBe("csharp");
    expect(markdownEditorLanguageFromPath("Assets/Scripts/PLAYER.CS#L12")).toBe("csharp");
  });

  it("parses C# source in the shared editor language compartment", () => {
    const state = EditorState.create({
      doc: "public sealed class Player : MonoBehaviour { private int health = 10; }",
      extensions: [markdownEditorLanguageExtension("csharp")],
    });
    const tree = syntaxTree(state);
    const highlightedTokens: Array<{ text: string; classes: string }> = [];
    highlightTree(tree, classHighlighter, (from, to, classes) => {
      highlightedTokens.push({ text: state.doc.sliceString(from, to), classes });
    });

    expect(tree.length).toBe(state.doc.length);
    expect(tree.toString()).toContain("TypeIdentifier");
    expect(highlightedTokens).toContainEqual({ text: "class", classes: "tok-keyword" });
    expect(highlightedTokens).toContainEqual({ text: "Player", classes: "tok-typeName" });
    expect(highlightedTokens).toContainEqual({ text: "10", classes: "tok-number" });
  });
});
