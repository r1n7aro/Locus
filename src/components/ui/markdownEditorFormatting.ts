const INLINE_COMPLETION_AT_CURSOR =
  /(?:(?<![\\*])\*\*\*[^*\n][\s\S]*?\*\*\*(?!\*)|(?<![\\_])___[^_\n][\s\S]*?___(?!_)|(?<![\\*])\*\*[^*\n][\s\S]*?\*\*(?!\*)|(?<![\\_])__[^_\n][\s\S]*?__(?!_)|(?<![\\*])\*[^*\n][\s\S]*?\*(?!\*)|(?<![\\_])_[^_\n][\s\S]*?_(?!_)|(?<![\\~])~~[^~\n][\s\S]*?~~(?!~)|(?<!\\)`[^`\n]+`)$/;
const INLINE_COMPLETION_GLOBAL =
  /(?<![\\*])\*\*\*[^*\n][\s\S]*?\*\*\*(?!\*)|(?<![\\_])___[^_\n][\s\S]*?___(?!_)|(?<![\\*])\*\*[^*\n][\s\S]*?\*\*(?!\*)|(?<![\\_])__[^_\n][\s\S]*?__(?!_)|(?<![\\*])\*[^*\n][\s\S]*?\*(?!\*)|(?<![\\_])_[^_\n][\s\S]*?_(?!_)|(?<![\\~])~~[^~\n][\s\S]*?~~(?!~)|(?<!\\)`[^`\n]+`/g;
const MARKDOWN_BLOCK_PREFIX_LINE_RE = /^\s*(?:[-*+]\s+|\d+[.)]\s+|>\s+|#{1,6}\s+)/;
const EXPLICIT_CODE_BLOCK_RE = /^(?:```|~~~)/;
const INDENTED_CODE_LINE_RE = /^(?: {4,}|\t)\S/;
const COMMAND_LINE_RE =
  /^\s*(?:\$|PS>|git|bun|npm|pnpm|yarn|cargo|python|node|uv|cd|ls|mkdir|rm|cp|mv|cat|echo|Get-|Set-)\b/i;
const CODE_TOKEN_RE =
  /\b(?:const|let|var|function|class|public|private|protected|return|if|else|for|while|switch|case|import|export|from|def|async|await|using|namespace|select|insert|update|delete|create|alter)\b|=>|===|!==|<\/?[a-z][^>]*>|[{}[\];]/i;

export function normalizeMarkdownEditorLineEndings(value: string): string {
  return value.replace(/\r\n/g, "\n").replace(/\u00a0/g, " ");
}

export function normalizeInlineMarkdown(value: string): string {
  return value.replace(/\\([*_`~])/g, "$1");
}

export function hasRenderableInlineCompletion(prefix: string): boolean {
  return INLINE_COMPLETION_AT_CURSOR.test(prefix);
}

export function countHiddenInlineMarkers(text: string): number {
  let hidden = 0;
  for (const match of text.matchAll(INLINE_COMPLETION_GLOBAL)) {
    const token = match[0];
    if (token.startsWith("***") || token.startsWith("___")) {
      hidden += 6;
      continue;
    }
    if (
      token.startsWith("**")
      || token.startsWith("__")
      || token.startsWith("~~")
    ) {
      hidden += 4;
      continue;
    }
    hidden += 2;
  }
  return hidden;
}

export function canSyncMarkdownEditorWhileFocused(currentMarkdown: string, nextMarkdown: string): boolean {
  return normalizeMarkdownEditorLineEndings(currentMarkdown) === normalizeMarkdownEditorLineEndings(nextMarkdown);
}

function htmlLooksLikePreformattedClipboardContent(html: string): boolean {
  const trimmed = html.trim();
  if (!trimmed) return false;

  if (typeof DOMParser === "undefined") {
    return /<pre[\s>]/i.test(trimmed)
      || /font-family\s*:\s*[^;"']*monospace/i.test(trimmed)
      || (/<table[\s>]/i.test(trimmed) && /line-number/i.test(trimmed) && /line-content/i.test(trimmed));
  }

  const doc = new DOMParser().parseFromString(trimmed, "text/html");
  const body = doc.body;
  const singleChild = body.childElementCount === 1 ? body.firstElementChild as HTMLElement | null : null;
  const preBlocks = body.querySelectorAll("pre");

  if (singleChild && /monospace/i.test(singleChild.style.fontFamily || "")) {
    return true;
  }
  if (singleChild && preBlocks.length === 1) {
    return true;
  }
  return singleChild?.tagName === "TABLE"
    && !!body.querySelector(".line-number")
    && !!body.querySelector(".line-content");
}

function plainTextLooksLikeCode(text: string): boolean {
  const trimmed = normalizeMarkdownEditorLineEndings(text).trim();
  if (!trimmed) return false;
  if (EXPLICIT_CODE_BLOCK_RE.test(trimmed)) return true;

  const nonEmptyLines = trimmed
    .split("\n")
    .map((line) => line.trimEnd())
    .filter((line) => line.trim().length > 0);
  if (!nonEmptyLines.length) return false;

  const codeLikeLineCount = nonEmptyLines.filter((line) => {
    const normalized = line.trim();
    return COMMAND_LINE_RE.test(normalized)
      || CODE_TOKEN_RE.test(normalized)
      || INDENTED_CODE_LINE_RE.test(line);
  }).length;

  if (codeLikeLineCount >= Math.min(2, nonEmptyLines.length)) return true;
  return codeLikeLineCount >= 1 && !MARKDOWN_BLOCK_PREFIX_LINE_RE.test(nonEmptyLines[0] ?? "");
}

function plainTextLooksLikeNarrativeMarkdown(text: string): boolean {
  const trimmed = normalizeMarkdownEditorLineEndings(text).trim();
  if (!trimmed) return false;

  const nonEmptyLines = trimmed
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  const markdownLineCount = nonEmptyLines.filter((line) => MARKDOWN_BLOCK_PREFIX_LINE_RE.test(line)).length;

  if (markdownLineCount >= 2) return true;
  if (markdownLineCount >= 1 && /[\p{L}\p{N}\p{Script=Han}]/u.test(trimmed)) return true;
  if (/[\p{Script=Han}]/u.test(trimmed)) return true;
  return /[A-Za-z]/.test(trimmed) && /[.!?]/.test(trimmed) && /\s/.test(trimmed);
}

export function shouldPreferMarkdownPlainTextPaste(html: string, text: string): boolean {
  const normalizedText = normalizeMarkdownEditorLineEndings(text).trim();
  if (!normalizedText || !htmlLooksLikePreformattedClipboardContent(html)) {
    return false;
  }
  if (plainTextLooksLikeCode(normalizedText)) {
    return false;
  }
  return plainTextLooksLikeNarrativeMarkdown(normalizedText);
}
