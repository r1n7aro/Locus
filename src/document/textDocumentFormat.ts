export type TextDocumentLineEnding = "\n" | "\r\n" | "\r";

/** The existing source editor's preferred newline policy; not a CSV lossless codec. */
export function detectTextDocumentLineEnding(text: string): TextDocumentLineEnding {
  return text.includes("\r\n") ? "\r\n" : text.includes("\r") ? "\r" : "\n";
}

/** Normalize newlines for text editors without trimming whitespace or blank records. */
export function normalizeTextDocumentLineEndings(text: string): string {
  return text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
}

export function serializeTextDocumentLineEndings(
  text: string,
  lineEnding: TextDocumentLineEnding,
): string {
  return normalizeTextDocumentLineEndings(text).replace(/\n/g, lineEnding);
}
