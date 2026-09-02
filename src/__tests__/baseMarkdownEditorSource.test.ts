import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();
const source = readFileSync(resolve(cwd, "src/components/ui/BaseMarkdownEditor.vue"), "utf8");
const extensionsSource = readFileSync(
  resolve(cwd, "src/components/ui/markdown-editor/codeMirrorMarkdownExtensions.ts"),
  "utf8",
);
const livePreviewSource = readFileSync(
  resolve(cwd, "src/components/ui/markdown-editor/markdownLivePreview.ts"),
  "utf8",
);

describe("BaseMarkdownEditor CodeMirror source", () => {
  it("mounts one persistent CodeMirror editing surface eagerly", () => {
    expect(source).toContain('import { EditorView } from "@codemirror/view"');
    expect(source).toContain("editorView = new EditorView({ state, parent })");
    expect(source).toContain('ref="mountRef" class="base-markdown-editor-host"');
    expect(source).not.toContain(["Vdi", "tor"].join(""));
    expect(source).not.toMatch(/MarkdownRenderer|SemanticCodeRenderer/);
    expect(source).not.toMatch(/<textarea|base-markdown-editor-textarea/);
    expect(source).not.toContain("v-if=");
  });

  it("reconfigures mode, language, read-only, and placeholder through compartments", () => {
    expect(source).toContain("const languageCompartment = new Compartment()");
    expect(source).toContain("const modeCompartment = new Compartment()");
    expect(source).toContain("const readOnlyCompartment = new Compartment()");
    expect(source).toContain("const placeholderCompartment = new Compartment()");
    expect(source).toContain("languageCompartment.reconfigure");
    expect(source).toContain("modeCompartment.reconfigure");
    expect(source).toContain("readOnlyCompartment.reconfigure");
    expect(source).toContain("placeholderCompartment.reconfigure");
    expect(extensionsSource).toContain('return "json"');
    expect(extensionsSource).toContain('return "python"');
    expect(extensionsSource).toContain("markdownLivePreview(livePreviewOptions)");
  });

  it("keeps rendered/native storage values as Live Preview/source modes", () => {
    expect(source).toContain("viewMode?: MarkdownEditorViewMode");
    expect(source).toContain('viewMode: "rendered"');
    expect(source).toContain("'is-rendered': viewMode === 'rendered'");
    expect(source).toContain("'is-source': viewMode === 'native'");
    expect(extensionsSource).toContain('viewMode === "rendered" && language === "markdown"');
    expect(extensionsSource).toContain('class: language === "markdown" ? "cm-source-mode"');
  });

  it("preserves the editor input contract and save/paste behavior", () => {
    expect(source).toContain('emit("update:modelValue"');
    expect(source).toContain('emit("shortcutSave")');
    expect(extensionsSource).toContain('key: "Mod-s"');
    expect(extensionsSource).toContain("history()");
    expect(extensionsSource).toContain("...historyKeymap");
    expect(extensionsSource).toContain('spellcheck: "false"');
    expect(extensionsSource).toContain("shouldPreferMarkdownPlainTextPaste");
    expect(extensionsSource).toContain("event.stopImmediatePropagation()");
    expect(extensionsSource).toContain("EditorView.lineWrapping");
  });

  it("applies external model values as minimal non-history changes", () => {
    expect(source).toContain("createMinimalTextChange(view.state.doc.toString(), normalized)");
    expect(source).toContain("Transaction.addToHistory.of(false)");
    expect(source).toContain("view.dispatch({");
    expect(source).not.toContain("setValue(");
  });

  it("restores document-local state, history, selection, and scroll from a bounded LRU", () => {
    expect(source).toContain("sessionCache?: MarkdownEditorSessionStore | null");
    expect(source).toContain("sessionPinned?: boolean");
    expect(source).toContain("const localSessionCache = new MarkdownEditorSessionCache()");
    expect(source).toContain("let activeSessionCache = props.sessionCache ?? localSessionCache");
    expect(source).toContain("StateEffect.reconfigure.of(editorExtensions())");
    expect(source).toContain("stateWithFreshConfiguration(state)");
    expect(source).toContain("stateWithFreshConfiguration(nextState)");
    expect(source).toContain("state: view.state");
    expect(source).toContain("pinned,");
    expect(source).toContain("scrollTop: currentScrollTop");
    expect(source).toContain("scrollLeft: currentScrollLeft");
    expect(source).toContain("function resolveScrollElement(view: EditorView): HTMLElement");
    expect(source).toContain('scrollElement.addEventListener("scroll", handleScroll, { passive: true })');
    expect(source).toContain("restoreTrackedScroll(editorView, scrollTop, scrollLeft)");
    expect(source).toContain("view.setState(nextState)");
    expect(source).toContain("localSessionCache.clear()");
    expect(source).not.toContain("props.sessionCache?.clear()");
  });

  it("suspends hidden editors while retaining their session", () => {
    expect(source).toContain("active?: boolean");
    expect(source).toContain("active: true");
    expect(source).toContain("function suspendEditor()");
    expect(source).toContain("saveActiveSession()");
    expect(source).toContain("editorView.destroy()");
    expect(source).toContain("mountEditor();");
  });

  it("builds Live Preview from visible syntax-tree ranges", () => {
    expect(livePreviewSource).toContain("syntaxTree(view.state)");
    expect(livePreviewSource).toContain("view.visibleRanges");
    expect(livePreviewSource).toContain('name === "StrongEmphasis"');
    expect(livePreviewSource).toContain('name === "TaskMarker"');
    expect(livePreviewSource).toContain('name === "Blockquote"');
    expect(livePreviewSource).toContain('name === "HorizontalRule"');
    expect(livePreviewSource).toContain('name === "FencedCode"');
  });

  it("uses one continuous CodeMirror layout for fixed and auto-grow editors", () => {
    expect(source).toMatch(/\.base-markdown-editor\s*\{[\s\S]*display:\s*flex;[\s\S]*flex:\s*1 1 0;[\s\S]*width:\s*100%;[\s\S]*min-height:\s*0;/);
    expect(source).toMatch(/\.base-markdown-editor :deep\(\.cm-scroller\)\s*\{[\s\S]*overflow:\s*auto;/);
    expect(source).toMatch(/\.base-markdown-editor\.auto-grow[\s\S]*height:\s*auto;/);
    expect(source).toMatch(/\.base-markdown-editor\.auto-grow :deep\(\.cm-scroller\)[\s\S]*overflow:\s*visible;[\s\S]*overscroll-behavior:\s*auto;/);
    expect(source).toContain("min-height: var(--markdown-editor-min-height);");
  });

  it("uses a low-opacity drawn selection that keeps text legible", () => {
    expect(extensionsSource).toContain('".cm-selectionBackground": {');
    expect(extensionsSource).toContain('"&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground"');
    expect(extensionsSource).toContain('background: "color-mix(in srgb, var(--accent-color) 18%, transparent)"');
    expect(extensionsSource).not.toContain(".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection");
  });

  it("keeps the editor surface visually unchanged while focused", () => {
    expect(source).not.toContain(".base-markdown-editor :deep(.cm-editor.cm-focused)");
    expect(extensionsSource).toMatch(/"&\.cm-focused":\s*\{\s*outline:\s*"none"/);
    expect(extensionsSource).not.toMatch(/"&\.cm-focused":\s*\{[^}]*boxShadow/);
  });
});
