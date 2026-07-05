// Display-only token estimate for prompt/knowledge payloads. A flat chars/4
// heuristic assumes ASCII-dominated text and undercounts CJK 4-6x (one CJK
// char is roughly one token, not a quarter). The backend keeps its own
// byte-based heuristic with usage calibration (src-tauri/src/compact.rs).

function isCjkCodePoint(cp: number): boolean {
  return (
    (cp >= 0x3000 && cp <= 0x30ff) || // CJK punctuation, hiragana, katakana
    (cp >= 0x3400 && cp <= 0x4dbf) || // CJK extension A
    (cp >= 0x4e00 && cp <= 0x9fff) || // CJK unified ideographs
    (cp >= 0xac00 && cp <= 0xd7af) || // Hangul syllables
    (cp >= 0xf900 && cp <= 0xfaff) || // CJK compatibility ideographs
    (cp >= 0xff00 && cp <= 0xffef) || // full-width and half-width forms
    (cp >= 0x20000 && cp <= 0x2ffff) // CJK extensions B-F
  );
}

export function estimateTextTokens(text: string): number {
  if (!text) return 0;
  let cjkChars = 0;
  let otherChars = 0;
  for (const ch of text) {
    if (isCjkCodePoint(ch.codePointAt(0) ?? 0)) cjkChars += 1;
    else otherChars += 1;
  }
  return cjkChars + Math.ceil(otherChars / 4);
}
