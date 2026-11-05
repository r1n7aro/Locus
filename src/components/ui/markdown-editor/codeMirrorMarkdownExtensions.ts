import type { Extension } from "@codemirror/state";
import { EditorState } from "@codemirror/state";
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import {
  HighlightStyle,
  indentUnit,
  syntaxHighlighting,
} from "@codemirror/language";
import { json } from "@codemirror/lang-json";
import { markdown, markdownKeymap } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";
import { csharp } from "@replit/codemirror-lang-csharp";
import { searchKeymap } from "@codemirror/search";
import {
  crosshairCursor,
  drawSelection,
  dropCursor,
  EditorView,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  placeholder,
  rectangularSelection,
} from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { GFM } from "@lezer/markdown";
import { markdownFootnoteSyntax } from "./markdownFootnoteSyntax";
import { normalizeMarkdownEditorLineEndings, shouldPreferMarkdownPlainTextPaste } from "../markdownEditorFormatting";
import type { MarkdownEditorViewMode } from "../markdownEditorViewMode";
import {
  markdownLivePreview,
  type MarkdownLivePreviewOptions,
} from "./markdownLivePreview";

export type MarkdownEditorLanguage = "markdown" | "json" | "python" | "csharp" | "plain";

export function markdownEditorLanguageFromPath(path: string): MarkdownEditorLanguage {
  const cleanPath = path.split(/[?#]/, 1)[0]?.toLowerCase() ?? "";
  if (cleanPath.endsWith(".json") || cleanPath.endsWith(".jsonc")) return "json";
  if (cleanPath.endsWith(".py") || cleanPath.endsWith(".pyw")) return "python";
  if (cleanPath.endsWith(".cs")) return "csharp";
  if (!cleanPath || cleanPath.endsWith(".md") || cleanPath.endsWith(".markdown")) return "markdown";
  return "plain";
}

export function markdownEditorLanguageExtension(language: MarkdownEditorLanguage): Extension {
  if (language === "json") return [json(), lineNumbers()];
  if (language === "python") return [python(), lineNumbers()];
  if (language === "csharp") return [csharp(), lineNumbers()];
  if (language === "plain") return lineNumbers();
  return markdown({
    extensions: [GFM, markdownFootnoteSyntax],
    addKeymap: false,
    completeHTMLTags: false,
  });
}

export function markdownEditorModeExtension(
  viewMode: MarkdownEditorViewMode,
  language: MarkdownEditorLanguage,
  livePreviewOptions: MarkdownLivePreviewOptions = {},
): Extension {
  if (viewMode === "rendered" && language === "markdown") {
    return markdownLivePreview(livePreviewOptions);
  }
  return EditorView.editorAttributes.of({
    class: language === "markdown" ? "cm-source-mode" : "cm-source-mode cm-code-document",
  });
}

export function markdownEditorReadOnlyExtension(disabled: boolean): Extension {
  return [
    EditorState.readOnly.of(disabled),
    EditorView.editable.of(!disabled),
  ];
}

export function markdownEditorPlaceholderExtension(value: string): Extension {
  return value ? placeholder(value) : [];
}

export function markdownEditorPasteExtension(): Extension {
  return EditorView.domEventHandlers({
    paste(event, view) {
      const html = event.clipboardData?.getData("text/html") ?? "";
      const text = event.clipboardData?.getData("text/plain") ?? "";
      if (view.state.readOnly || !shouldPreferMarkdownPlainTextPaste(html, text)) {
        return false;
      }

      event.preventDefault();
      event.stopPropagation();
      event.stopImmediatePropagation();
      view.dispatch(view.state.replaceSelection(normalizeMarkdownEditorLineEndings(text)));
      return true;
    },
  });
}

const locusHighlightStyle = HighlightStyle.define([
  { tag: tags.heading, color: "var(--text-color)", fontWeight: "600" },
  { tag: tags.strong, fontWeight: "600" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.strikethrough, textDecoration: "line-through" },
  { tag: tags.link, color: "var(--accent-color)" },
  { tag: tags.url, color: "var(--text-secondary)" },
  { tag: tags.monospace, color: "var(--text-color)" },
  { tag: tags.comment, color: "var(--text-secondary)", fontStyle: "italic" },
  { tag: tags.keyword, color: "var(--md-syntax-keyword, var(--accent-color))" },
  { tag: tags.string, color: "var(--md-syntax-string, var(--status-success-fg, var(--text-color)))" },
  { tag: tags.number, color: "var(--md-syntax-number, var(--status-warn-fg, var(--text-color)))" },
  { tag: [tags.bool, tags.null], color: "var(--md-syntax-literal, var(--accent-color))" },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], color: "var(--md-syntax-function, var(--text-color))" },
  { tag: [tags.typeName, tags.className], color: "var(--md-syntax-type, var(--text-color))" },
]);

const locusEditorTheme = EditorView.theme({
  "&": {
    height: "100%",
    minHeight: "0",
    color: "var(--text-color)",
    backgroundColor: "transparent",
  },
  "&.cm-focused": {
    outline: "none",
  },
  ".cm-scroller": {
    fontFamily: "var(--font-prose)",
  },
  ".cm-content": {
    caretColor: "var(--accent-color)",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeftColor: "var(--accent-color)",
  },
  ".cm-selectionBackground": {
    background: "color-mix(in srgb, var(--accent-color) 18%, transparent)",
  },
  // Chromium can repaint the whole text fragment twice when a fully selected
  // line becomes partially selected with a transparent native selection.
  // Keep 1/255 alpha (smaller values can round to zero); drawSelection still
  // supplies the visible background. The extra .cm-content beats its hiding
  // rule without overriding the native Highlight rule for focused widgets.
  ".cm-content .cm-line::selection, .cm-content .cm-line ::selection": {
    backgroundColor: "#00000001 !important",
  },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground": {
    background: "color-mix(in srgb, var(--accent-color) 18%, transparent)",
  },
  ".cm-gutters": {
    color: "var(--text-secondary)",
    backgroundColor: "color-mix(in srgb, var(--sidebar-bg) 58%, transparent)",
    borderRight: "1px solid color-mix(in srgb, var(--border-color) 76%, transparent)",
  },
  ".cm-placeholder": {
    color: "var(--text-secondary)",
    opacity: "0.55",
  },
  ".cm-panels": { backgroundColor: "var(--panel-bg)", color: "var(--text-color)" },
  ".cm-panels-top": { borderBottom: "none" },
  ".cm-searchMatch": { backgroundColor: "var(--find-match-bg)", outline: "none", borderRadius: "2px" },
  ".cm-searchMatch-selected": { backgroundColor: "var(--find-match-current-bg)" },
});

export function markdownEditorBaseExtensions(onSave: () => void): Extension[] {
  return [
    highlightSpecialChars(),
    history(),
    drawSelection(),
    dropCursor(),
    rectangularSelection(),
    crosshairCursor(),
    EditorState.allowMultipleSelections.of(true),
    EditorState.tabSize.of(2),
    indentUnit.of("  "),
    EditorView.lineWrapping,
    EditorView.contentAttributes.of({
      spellcheck: "false",
      autocorrect: "off",
      autocapitalize: "off",
    }),
    keymap.of([
      {
        key: "Mod-s",
        preventDefault: true,
        run: () => {
          onSave();
          return true;
        },
      },
      indentWithTab,
      ...markdownKeymap,
      ...defaultKeymap,
      ...historyKeymap,
      // Opening search is routed through the configurable, focus-aware shortcut.
      ...searchKeymap.filter((binding) => binding.key !== "Mod-f"),
    ]),
    markdownEditorPasteExtension(),
    syntaxHighlighting(locusHighlightStyle),
    locusEditorTheme,
  ];
}
