import hljs, { langFromPath } from "../hljs";
import { renderHighlightedCodeLines } from "./markdownCodeLines";

export type SemanticCodeLanguage = "python" | "json";

const SEMANTIC_CODE_LANGUAGES = new Set<string>(["python", "json"]);

export type SemanticCodeDisplay = {
  content: string;
  parseError: string | null;
};

export function semanticCodeLanguageFromPath(filePath?: string | null): SemanticCodeLanguage | null {
  const language = filePath ? langFromPath(filePath) : null;
  if (!language || !SEMANTIC_CODE_LANGUAGES.has(language)) return null;
  return language as SemanticCodeLanguage;
}

export function normalizeSemanticCodeForDisplay(
  content: string,
  language: SemanticCodeLanguage,
): SemanticCodeDisplay {
  if (language !== "json") {
    return { content, parseError: null };
  }

  try {
    return {
      content: JSON.stringify(JSON.parse(content), null, 2),
      parseError: null,
    };
  } catch (error) {
    return {
      content,
      parseError: error instanceof Error ? error.message : String(error),
    };
  }
}

export function renderSemanticCodeHtml(
  content: string,
  language: SemanticCodeLanguage,
): string {
  const display = normalizeSemanticCodeForDisplay(content, language);
  let highlighted = escapeHtml(display.content);

  if (hljs.getLanguage(language)) {
    try {
      highlighted = hljs.highlight(display.content, { language }).value;
    } catch {
      highlighted = escapeHtml(display.content);
    }
  }

  return renderHighlightedCodeLines(highlighted);
}

function escapeHtml(source: string): string {
  return source.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
