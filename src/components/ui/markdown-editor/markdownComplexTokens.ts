export type MarkdownTableAlignment = "left" | "center" | "right" | null;

export interface MarkdownTableModel {
  header: string[];
  rows: string[][];
  alignments: MarkdownTableAlignment[];
}

export interface MarkdownMathToken {
  from: number;
  to: number;
  latex: string;
  display: boolean;
  block: boolean;
  openingLength: number;
}

export type MarkdownReferenceKind =
  | "knowledge"
  | "unity-asset"
  | "unity-scene-object"
  | "workspace"
  | "file"
  | "view"
  | "unity-property";

export interface MarkdownReferenceToken {
  from: number;
  to: number;
  raw: string;
  path: string;
  label: string;
  kind: MarkdownReferenceKind;
  line?: number;
  level?: string;
}

const KNOWLEDGE_PATH_RE = /^(?:Locus\/knowledge\/)?(design|plan|memory|skill|reference)\/(.+\.md)$/i;
const WINDOWS_ABSOLUTE_RE = /^[A-Za-z]:[\\/]/;
const UNC_ABSOLUTE_RE = /^(?:\\\\|\/\/)[^\\/]+[\\/][^\\/]+/;
const POSIX_ABSOLUTE_RE = /^\/(?!\/)/;
const WORKSPACE_ROOT_RE = /^(?:src|src-tauri|ProjectSettings|Editor|Library)\//i;
const UNITY_ROOT_RE = /^(?:Assets|Packages)\//i;
const UNITY_SCENE_OBJECT_RE = /^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i;
const GENERIC_FILE_RE = /^(?:[^/\r\n]+\/)+[^/\r\n]+\.[A-Za-z0-9][^/\r\n]*$/;
const UNITY_PREFIX_RE = /^(?:asset|unity|ref)(?::([A-Za-z-]+))?\s+(.+)$/i;
const UNITY_SUFFIX_RE = /^(.+?)\s+\|\s*([A-Za-z-]+)$/;
const LINE_SUFFIX_RE = /^(.+?)(?::(\d+)|#L(\d+)|#fileID:-?\d+)?$/i;

function splitTableRow(source: string): string[] {
  const trimmed = source.trim();
  const cells: string[] = [];
  let cell = "";
  let escaped = false;
  let codeTicks = 0;

  for (let index = 0; index < trimmed.length; index += 1) {
    const char = trimmed[index];
    if (escaped) {
      cell += char;
      escaped = false;
      continue;
    }
    if (char === "\\") {
      escaped = true;
      cell += char;
      continue;
    }
    if (char === "`") {
      let run = 1;
      while (trimmed[index + run] === "`") run += 1;
      if (codeTicks === 0) codeTicks = run;
      else if (codeTicks === run) codeTicks = 0;
      cell += "`".repeat(run);
      index += run - 1;
      continue;
    }
    if (char === "|" && codeTicks === 0) {
      cells.push(cell.trim());
      cell = "";
      continue;
    }
    cell += char;
  }
  cells.push(cell.trim());

  if (trimmed.startsWith("|")) cells.shift();
  if (trimmed.endsWith("|") && !trimmed.endsWith("\\|")) cells.pop();
  return cells;
}

function tableAlignment(source: string): MarkdownTableAlignment | undefined {
  const trimmed = source.trim();
  if (!/^:?-{3,}:?$/.test(trimmed)) return undefined;
  if (trimmed.startsWith(":") && trimmed.endsWith(":")) return "center";
  if (trimmed.endsWith(":")) return "right";
  if (trimmed.startsWith(":")) return "left";
  return null;
}

export function displayMarkdownTableCell(source: string): string {
  return source
    .replace(/\\\|/g, "|")
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/(\*\*|__|~~|`)/g, "")
    .trim();
}

export function parseMarkdownTable(source: string): MarkdownTableModel | null {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  if (lines.length < 2) return null;

  const header = splitTableRow(lines[0]);
  const delimiter = splitTableRow(lines[1]);
  if (!header.length || delimiter.length !== header.length) return null;

  const alignments = delimiter.map(tableAlignment);
  if (alignments.some((alignment) => alignment === undefined)) return null;

  const normalizeRow = (line: string) => {
    const cells = splitTableRow(line);
    if (cells.length > header.length) return null;
    while (cells.length < header.length) cells.push("");
    return cells.map(displayMarkdownTableCell);
  };

  const rows: string[][] = [];
  for (const line of lines.slice(2)) {
    const row = normalizeRow(line);
    if (!row) return null;
    rows.push(row);
  }

  return {
    header: header.map(displayMarkdownTableCell),
    rows,
    alignments: alignments as MarkdownTableAlignment[],
  };
}

function isEscaped(source: string, index: number): boolean {
  let slashes = 0;
  for (let cursor = index - 1; cursor >= 0 && source[cursor] === "\\"; cursor -= 1) {
    slashes += 1;
  }
  return slashes % 2 === 1;
}

function findUnescaped(source: string, marker: string, from: number, limit = source.length): number {
  let cursor = from;
  while (cursor < limit) {
    const found = source.indexOf(marker, cursor);
    if (found < 0 || found >= limit) return -1;
    if (!isEscaped(source, found)) return found;
    cursor = found + marker.length;
  }
  return -1;
}

function isBlockDelimited(source: string, from: number, to: number): boolean {
  const lineStart = source.lastIndexOf("\n", from - 1) + 1;
  const nextLineBreak = source.indexOf("\n", to);
  const lineEnd = nextLineBreak < 0 ? source.length : nextLineBreak;
  return /^ {0,3}$/.test(source.slice(lineStart, from))
    && /^[ \t]*$/.test(source.slice(to, lineEnd));
}

/**
 * Lightweight math scanner matching the four syntaxes supported by the
 * read-only Markdown renderer. Callers exclude code/image/link ranges using
 * the Lezer tree before creating widgets.
 */
export function findMarkdownMathTokens(source: string, baseOffset = 0): MarkdownMathToken[] {
  const tokens: MarkdownMathToken[] = [];
  let cursor = 0;

  while (cursor < source.length) {
    const char = source[cursor];
    if (char === "$" && !isEscaped(source, cursor)) {
      if (source.startsWith("$$", cursor)) {
        const close = findUnescaped(source, "$$", cursor + 2, Math.min(source.length, cursor + 10_000));
        if (close > cursor + 2) {
          const latex = source.slice(cursor + 2, close).trim();
          if (latex) {
            const block = isBlockDelimited(source, cursor, close + 2);
            tokens.push({
              from: baseOffset + cursor,
              to: baseOffset + close + 2,
              latex,
              display: true,
              block,
              openingLength: 2,
            });
            cursor = close + 2;
            continue;
          }
        }
      } else {
        const lineEnd = source.indexOf("\n", cursor + 1);
        const limit = Math.min(lineEnd < 0 ? source.length : lineEnd, cursor + 122);
        const close = findUnescaped(source, "$", cursor + 1, limit + 1);
        if (close > cursor + 1 && !/\d/.test(source[close + 1] ?? "")) {
          const body = source.slice(cursor + 1, close);
          if (body.length <= 120 && !/^\s|\s$/.test(body)) {
            tokens.push({
              from: baseOffset + cursor,
              to: baseOffset + close + 1,
              latex: body,
              display: false,
              block: false,
              openingLength: 1,
            });
            cursor = close + 1;
            continue;
          }
        }
      }
    }

    if (char === "\\" && !isEscaped(source, cursor)) {
      const delimiter = source[cursor + 1];
      if (delimiter === "(" || delimiter === "[") {
        const closeMarker = delimiter === "(" ? "\\)" : "\\]";
        const close = findUnescaped(source, closeMarker, cursor + 2, Math.min(source.length, cursor + 10_000));
        if (close > cursor + 2) {
          const body = source.slice(cursor + 2, close);
          const display = delimiter === "[";
          const blockDelimited = isBlockDelimited(source, cursor, close + 2);
          if (body.trim() && (!display || blockDelimited || /[\\^_={}+]/.test(body))) {
            tokens.push({
              from: baseOffset + cursor,
              to: baseOffset + close + 2,
              latex: body.trim(),
              display,
              block: blockDelimited,
              openingLength: 2,
            });
            cursor = close + 2;
            continue;
          }
        }
      }
    }

    cursor += 1;
  }

  return tokens;
}

function stripInlineCode(source: string): string {
  const match = source.match(/^(`+)([\s\S]*)\1$/);
  return match ? match[2].trim() : source.trim();
}

function displayPathLabel(path: string): string {
  const clean = path.replace(/[#:]L?\d+$/i, "").replace(/[\\/]+$/, "");
  const segments = clean.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] || clean || path;
}

function referenceFromValue(
  rawValue: string,
  from: number,
  to: number,
): MarkdownReferenceToken | null {
  const rawTrimmed = rawValue.trim();
  const explicitReference = rawTrimmed.startsWith("`")
    || rawTrimmed.startsWith("{")
    || rawTrimmed.startsWith("@")
    || rawTrimmed.startsWith("\"")
    || rawTrimmed.startsWith("'");
  let value = stripInlineCode(rawValue).trim();
  let level: string | undefined;
  const prefixed = value.match(UNITY_PREFIX_RE);
  if (prefixed) {
    level = prefixed[1]?.toLowerCase();
    value = prefixed[2].trim();
  }
  const suffixed = value.match(UNITY_SUFFIX_RE);
  if (suffixed) {
    level = suffixed[2].toLowerCase();
    value = suffixed[1].trim();
  }
  if (value.startsWith("{") && value.endsWith("}")) value = value.slice(1, -1).trim();
  value = value.replace(/^@(?=[^@\r\n]*[\\/])/, "").replace(/\\/g, "/");

  if (/^view:/i.test(value)) {
    const path = value.slice(5).replace(/^\{|\}$/g, "").trim();
    if (!path || /[<>\r\n]/.test(path)) return null;
    return { from, to, raw: rawValue, path, label: path, kind: "view" };
  }

  const suffix = value.match(LINE_SUFFIX_RE);
  if (!suffix) return null;
  const path = suffix[1].trim().replace(/\\/g, "/");
  const line = Number(suffix[2] || suffix[3] || 0) || undefined;
  const knowledge = path.match(KNOWLEDGE_PATH_RE);
  if (knowledge) {
    const normalizedPath = `${knowledge[1].toLowerCase()}/${knowledge[2]}`;
    return {
      from,
      to,
      raw: rawValue,
      path: normalizedPath,
      label: displayPathLabel(normalizedPath),
      kind: "knowledge",
      line,
    };
  }

  const sceneObject = path.match(UNITY_SCENE_OBJECT_RE);
  if (sceneObject) {
    const objectSegments = sceneObject[2].split("/").filter(Boolean);
    return {
      from,
      to,
      raw: rawValue,
      path,
      label: objectSegments[objectSegments.length - 1] || displayPathLabel(path),
      kind: "unity-scene-object",
      line,
      level,
    };
  }
  if (UNITY_ROOT_RE.test(path)) {
    if (!explicitReference && !/[\\/]$|\.[^/\\]+(?:#fileID:-?\d+)?$/i.test(path)) return null;
    return {
      from,
      to,
      raw: rawValue,
      path,
      label: displayPathLabel(path),
      kind: "unity-asset",
      line,
      level,
    };
  }
  if (WORKSPACE_ROOT_RE.test(path)) {
    if (!explicitReference && !/[\\/]$|\.[^/\\]+$/i.test(path)) return null;
    return { from, to, raw: rawValue, path, label: displayPathLabel(path), kind: "workspace", line };
  }
  if (WINDOWS_ABSOLUTE_RE.test(path) || UNC_ABSOLUTE_RE.test(path) || POSIX_ABSOLUTE_RE.test(path)) {
    return { from, to, raw: rawValue, path, label: displayPathLabel(path), kind: "file", line };
  }
  if (GENERIC_FILE_RE.test(path)) {
    return { from, to, raw: rawValue, path, label: displayPathLabel(path), kind: "file", line };
  }
  return null;
}

export function parseInlineMarkdownReference(
  source: string,
  from = 0,
): MarkdownReferenceToken | null {
  return referenceFromValue(source, from, from + source.length);
}

function trimReferenceTrailingPunctuation(source: string): string {
  return source.replace(/[.,;，。；、？！\])}）】》」』]+$/, "");
}

/** Finds conservative plain-text references. Inline code and fenced code are
 * parsed from their syntax nodes by the Live Preview plugin. */
export function findPlainMarkdownReferences(source: string, baseOffset = 0): MarkdownReferenceToken[] {
  const refs: MarkdownReferenceToken[] = [];
  const occupied: Array<{ from: number; to: number }> = [];
  const add = (token: MarkdownReferenceToken | null) => {
    if (!token || occupied.some((range) => token.from < range.to && token.to > range.from)) return;
    occupied.push(token);
    refs.push(token);
  };

  const lines = source.split("\n");
  let lineOffset = 0;
  for (const line of lines) {
    const viewMatch = line.match(/^\s*`?view:([A-Za-z0-9][A-Za-z0-9._/-]{0,127}|\{[^<>{}\r\n]{1,160}\})`?\s*$/i);
    if (viewMatch) {
      const start = line.indexOf(viewMatch[0].trim());
      add(referenceFromValue(
        viewMatch[0].trim(),
        baseOffset + lineOffset + start,
        baseOffset + lineOffset + start + viewMatch[0].trim().length,
      ));
      lineOffset += line.length + 1;
      continue;
    }

    const knowledgeRe = /(?:Locus\/knowledge\/)?(?:design|plan|memory|skill|reference)\/[\w.\/-]+\.md/gi;
    for (const match of line.matchAll(knowledgeRe)) {
      const start = match.index ?? 0;
      add(referenceFromValue(
        match[0],
        baseOffset + lineOffset + start,
        baseOffset + lineOffset + start + match[0].length,
      ));
    }

    const rootRe = /(?:^|[\s([{"'])((?:\{@?|@)?(?:Assets|Packages|src-tauri|src|ProjectSettings|Editor|Library)\/)/gi;
    for (const match of line.matchAll(rootRe)) {
      const prefix = match[0].slice(0, match[0].length - match[1].length);
      const start = (match.index ?? 0) + prefix.length;
      const braced = line.slice(start).startsWith("{");
      let end: number;
      if (braced) {
        const close = line.indexOf("}", start);
        if (close < 0) continue;
        end = close + 1;
      } else {
        const rest = line.slice(start);
        const sceneMarker = rest.toLowerCase().indexOf(".unity/");
        const token = sceneMarker >= 0
          ? rest.split(/[<>"'`{}，。；、？！]/, 1)[0]
          : rest.split(/[\s<>"'`{}，。；、？！]/, 1)[0];
        end = start + trimReferenceTrailingPunctuation(token).length;
      }
      const raw = line.slice(start, end);
      add(referenceFromValue(raw, baseOffset + lineOffset + start, baseOffset + lineOffset + end));
    }
    lineOffset += line.length + 1;
  }

  return refs.sort((a, b) => a.from - b.from || a.to - b.to);
}

export function isUnityReferenceFenceLanguage(language: string): boolean {
  return /^(?:asset|unity|ref)(?::|-)?(?:inline|chip|row|line|block|preview|thumbnail|thumb|inspector|inspect|editor|edit|editable)?$/i.test(language.trim());
}

export function isUnityPropertyFenceLanguage(language: string): boolean {
  return /^(?:unity[_-]?property|unity:property|property:unity|unity-property-editor)$/i.test(language.trim());
}
