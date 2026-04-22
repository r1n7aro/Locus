const BLOCKQUOTE_PREFIX_RE = /^(\s*(?:>\s*)+)/;
const PUNCTUATION_TERMINATED_STRONG_RE =
  /((?:\*\*[^*\n]*[：:；;，,。.!！？?、）】》」』]\*\*)|(?:__[^_\n]*[：:；;，,。.!！？?、）】》」』]__))(?=[\p{L}\p{N}\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\[(（【「『<])/gu;

function blockquotePrefix(line: string): string | null {
  const match = line.match(BLOCKQUOTE_PREFIX_RE);
  return match?.[1]?.trimEnd() || null;
}

function normalizeLooseBlockquotes(markdown: string): string {
  const lines = markdown.split("\n");
  for (let index = 1; index < lines.length - 1; index += 1) {
    if (lines[index].trim() !== "") continue;
    if (lines[index - 1].trim() === "" || lines[index + 1].trim() === "") continue;

    const previousPrefix = blockquotePrefix(lines[index - 1]);
    const nextPrefix = blockquotePrefix(lines[index + 1]);
    if (!previousPrefix || !nextPrefix) continue;

    lines[index] = previousPrefix;
  }
  return lines.join("\n");
}

function normalizeStrongLabelSpacing(markdown: string): string {
  return markdown.replace(PUNCTUATION_TERMINATED_STRONG_RE, "$1 ");
}

export function normalizeMarkdownForRender(markdown: string): string {
  if (!markdown) return "";
  const normalizedLineEndings = markdown.replace(/\r\n/g, "\n");
  return normalizeStrongLabelSpacing(normalizeLooseBlockquotes(normalizedLineEndings));
}
