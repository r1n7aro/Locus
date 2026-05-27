import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

const source = readFileSync(resolve(cwd, "src/components/ui/BaseMarkdownEditor.vue"), "utf8");

describe("BaseMarkdownEditor source", () => {
  it("wraps Vditor in Typora-like instant rendering mode", () => {
    expect(source).toContain("import Vditor from \"vditor\"");
    expect(source).toContain("mode: \"ir\"");
    expect(source).toContain("height: MARKDOWN_EDITOR_PANEL_HEIGHT");
    expect(source).toContain("maxWidth: MARKDOWN_EDITOR_PANEL_MAX_WIDTH");
    expect(source).toContain("toolbar: []");
    expect(source).toContain("cache: {");
    expect(source).toContain("enable: false");
  });

  it("supports a shared native markdown source view", () => {
    expect(source).toContain("viewMode?: MarkdownEditorViewMode;");
    expect(source).toContain("contentPath?: string;");
    expect(source).toContain("viewMode: \"rendered\"");
    expect(source).toContain("contentPath: \"\"");
    expect(source).toContain("const isNativeMode = computed(() => props.viewMode === \"native\")");
    expect(source).toContain("const isReadonlyRenderedMode = computed(() => props.disabled && props.viewMode === \"rendered\")");
    expect(source).toContain("const readonlyCodeLanguage = computed(() =>");
    expect(source).toContain("const shouldUseVditor = computed(() => !isNativeMode.value && !isReadonlyRenderedMode.value)");
    expect(source).toContain("class=\"base-markdown-editor-textarea\"");
    expect(source).toContain("font-family: var(--font-mono-editor);");
    expect(source).toContain("function handleNativeInput(event: Event)");
    expect(source).toContain("function handleNativeKeydown(event: KeyboardEvent)");
    expect(source).toContain("spellcheck=\"false\"");
  });

  it("keeps the save shortcut and theme sync inside the wrapper", () => {
    expect(source).toContain("event.key.toLowerCase() === \"s\"");
    expect(source).toContain("emit(\"shortcutSave\")");
    expect(source).toContain("editor.setTheme(isDarkTheme() ? \"dark\" : \"classic\")");
    expect(source).toContain("new MutationObserver(() => applyTheme())");
    expect(source).toContain("createMarkdownEditorResizeSync(mountRef.value, syncPanelLayout)");
  });

  it("intercepts prose pasted from preformatted html before Vditor auto-wraps it as code", () => {
    expect(source).toContain("shouldPreferMarkdownPlainTextPaste");
    expect(source).toContain("event.stopImmediatePropagation()");
    expect(source).toContain("editor.insertMD(text)");
    expect(source).toContain("editable.addEventListener(\"paste\", onPaste, true)");
  });

  it("uses the editor surface as the only scroll container", () => {
    expect(source).toMatch(/\.base-markdown-editor\s*\{[\s\S]*display:\s*flex;[\s\S]*min-height:\s*0;/);
    expect(source).toMatch(/\.base-markdown-editor-native\s*\{[\s\S]*display:\s*flex;[\s\S]*min-height:\s*0;/);
    expect(source).toMatch(/\.base-markdown-editor-rendered\s*\{[\s\S]*overflow:\s*auto;/);
    expect(source).toMatch(/\.base-markdown-editor-textarea\s*\{[\s\S]*overflow:\s*auto;/);
    expect(source).toMatch(/\.base-markdown-editor\s*:deep\(\.vditor-content\)\s*\{[\s\S]*overflow:\s*hidden;/);
    expect(source).toMatch(/\.base-markdown-editor\s*:deep\(\.vditor-ir\)\s*\{[\s\S]*padding:\s*0 !important;[\s\S]*overflow:\s*hidden;/);
    expect(source).toMatch(/\.base-markdown-editor\s*:deep\(\.vditor-ir pre\.vditor-reset\)\s*\{[\s\S]*height:\s*auto;[\s\S]*overflow:\s*auto;/);
  });

  it("keeps a compact left gutter for knowledge editing", () => {
    expect(source).toContain("padding: 14px 14px 16px 16px !important;");
    expect(source).toContain("padding-left: 14px;");
    expect(source).toContain("content: none;");
    expect(source).toContain("display: none;");
  });

  it("uses a neutral cursor for disabled read-only content", () => {
    expect(source).toMatch(/\.base-markdown-editor\.disabled\s*\{[\s\S]*cursor:\s*default;/);
    expect(source).toMatch(/pre\.vditor-reset\[contenteditable="false"\]\)\s*\{[\s\S]*cursor:\s*default;/);
    expect(source).toContain("import MarkdownRenderer from \"../MarkdownRenderer.vue\"");
    expect(source).toContain("import SemanticCodeRenderer from \"./SemanticCodeRenderer.vue\"");
    expect(source).toContain("semanticCodeLanguageFromPath(props.contentPath)");
    expect(source).toContain("<SemanticCodeRenderer");
    expect(source).toContain("<MarkdownRenderer v-else :content=\"modelValue\" />");
    expect(source).not.toContain("cursor: wait;");
  });
});
