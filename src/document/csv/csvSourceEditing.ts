import { buildDocumentTextHunks } from "../documentText";
import { normalizeTextDocumentLineEndings } from "../textDocumentFormat";

/** Native textarea/CodeMirror normalize CRLF. Preserve untouched source spans when applying their edits. */
export function applyCsvNormalizedTextEdit(raw: string, edited: string, newline: string): string {
  const normalized = normalizeTextDocumentLineEndings(raw);
  if (normalized === edited) return raw;
  const changes = buildDocumentTextHunks(normalized, edited);
  const offsets: number[] = [];
  for (let index = 0; index < raw.length; index++) {
    offsets.push(index);
    if (raw[index] === "\r" && raw[index + 1] === "\n") index++;
  }
  offsets.push(raw.length);
  let next = raw;
  for (const hunk of changes.reverse()) next = next.slice(0, offsets[hunk.start])
    + hunk.newText.replace(/\n/g, newline) + next.slice(offsets[hunk.end]);
  return next;
}
