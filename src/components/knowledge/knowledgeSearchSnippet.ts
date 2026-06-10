export interface KnowledgeSnippetSegment {
  text: string;
  highlighted: boolean;
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function hasCjk(text: string): boolean {
  // Hiragana/Katakana, CJK unified ideographs (+ext A), compatibility block.
  return /[぀-ヿ㐀-䶿一-鿿豈-﫿]/.test(text);
}

/**
 * Split a search snippet into plain / highlighted segments. Prefers the
 * backend's matchedTerms; falls back to whitespace tokens of the raw query.
 * Single-letter latin tokens are ignored so a query like "a tool" does not
 * light up every "a" in the snippet.
 */
export function buildKnowledgeSnippetSegments(
  snippet: string | null | undefined,
  matchedTerms: readonly string[] | null | undefined,
  query: string,
): KnowledgeSnippetSegment[] {
  const text = (snippet ?? "").trim();
  if (!text) return [];

  const terms = new Set<string>();
  for (const term of matchedTerms ?? []) {
    const trimmed = term.trim();
    if (trimmed) terms.add(trimmed);
  }
  if (!terms.size) {
    for (const token of query.trim().split(/\s+/)) {
      if (!token) continue;
      if (token.length >= 2 || hasCjk(token)) terms.add(token);
    }
  }

  const patterns = Array.from(terms)
    .sort((left, right) => right.length - left.length)
    .map(escapeRegExp);
  if (!patterns.length) return [{ text, highlighted: false }];

  const matcher = new RegExp(patterns.join("|"), "giu");
  const segments: KnowledgeSnippetSegment[] = [];
  let cursor = 0;
  for (const match of text.matchAll(matcher)) {
    const start = match.index ?? 0;
    const value = match[0] ?? "";
    if (!value) continue;
    if (start > cursor) {
      segments.push({ text: text.slice(cursor, start), highlighted: false });
    }
    segments.push({ text: value, highlighted: true });
    cursor = start + value.length;
  }
  if (cursor < text.length) {
    segments.push({ text: text.slice(cursor), highlighted: false });
  }
  return segments.length ? segments : [{ text, highlighted: false }];
}
