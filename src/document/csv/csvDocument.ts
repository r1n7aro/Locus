import Papa from "papaparse";

export const CSV_MAX_CELLS = 500_000;
export const CSV_MAX_CHARACTERS = 16 * 1024 * 1024;
export interface CsvField { value: string; raw: string }
export interface CsvRecord { fields: CsvField[]; ending: string; start: number }
export interface CsvDocument {
  bom: string;
  delimiter: string;
  newline: string;
  records: CsvRecord[];
  columnCount: number;
  byteLength: number;
}
export interface CsvCellEdit { row: number; column: number; value: string }
export interface CsvCellEditResult { document: CsvDocument; text: string }
const csvEditOrigins = new WeakMap<CsvDocument, { previous: CsvDocument; rows: number[] }>();

export class CsvParseError extends Error {
  constructor(message: string, readonly offset: number) { super(message); }
}

/** Preserve lexical fields and record terminators; Papa supplies dialect detection. */
export function parseCsvDocument(source: string, delimiter?: string): CsvDocument {
  const byteLength = new TextEncoder().encode(source).byteLength;
  if (source.length > CSV_MAX_CHARACTERS || byteLength > CSV_MAX_CHARACTERS) throw new CsvParseError("csv.tooLarge", 0);
  const bom = source.startsWith("\ufeff") ? "\ufeff" : "";
  const detected = delimiter ?? Papa.parse<string[]>(source.slice(bom.length), {
    preview: 10, dynamicTyping: false, header: false, skipEmptyLines: false,
    delimitersToGuess: [",", ";", "\t", "|"],
  }).meta.delimiter ?? ",";
  if (![",", ";", "\t", "|"].includes(detected)) throw new CsvParseError("csv.invalidDelimiter", 0);
  const records: CsvRecord[] = [];
  let position = bom.length;
  let columnCount = 0;
  let cellCount = 0;
  let newline = "";
  while (position < source.length) {
    const record: CsvRecord = { fields: [], ending: "", start: position };
    while (true) {
      const start = position;
      let value = "";
      if (source[position] === '"') {
        position++;
        let closed = false;
        while (position < source.length) {
          const char = source[position++]!;
          if (char !== '"') { value += char; continue; }
          if (source[position] === '"') { value += '"'; position++; continue; }
          closed = true;
          break;
        }
        if (!closed) throw new CsvParseError("csv.unclosedQuote", start);
        if (position < source.length && ![detected, "\n", "\r"].includes(source[position]!)) {
          throw new CsvParseError("csv.invalidQuote", position);
        }
      } else {
        while (position < source.length && ![detected, "\n", "\r"].includes(source[position]!)) {
          if (source[position] === '"') throw new CsvParseError("csv.invalidQuote", position);
          position++;
        }
        value = source.slice(start, position);
      }
      record.fields.push({ value, raw: source.slice(start, position) });
      if (++cellCount > CSV_MAX_CELLS) throw new CsvParseError("csv.tooLarge", position);
      if (source[position] === detected) { position++; continue; }
      if (source[position] === "\r") {
        record.ending = source[position + 1] === "\n" ? "\r\n" : "\r";
      } else if (source[position] === "\n") record.ending = "\n";
      position += record.ending.length;
      if (!newline && record.ending) newline = record.ending;
      break;
    }
    columnCount = Math.max(columnCount, record.fields.length);
    records.push(record);
    if (columnCount > 10000 || records.length * columnCount > CSV_MAX_CELLS) throw new CsvParseError("csv.tooLarge", position);
  }
  return { bom, delimiter: detected, newline: newline || "\n", records, columnCount, byteLength };
}

export function csvField(value: string, delimiter: string, quoted = false): CsvField {
  const raw = quoted || value.includes(delimiter) || /["\r\n]/.test(value)
    ? `"${value.replace(/"/g, '""')}"` : value;
  return { value, raw };
}

export function serializeCsvDocument(document: CsvDocument): string {
  return document.bom + document.records.map((row) =>
    row.fields.map((field) => field.raw).join(document.delimiter) + row.ending).join("");
}

function assertCsvCellEdit({ row, column }: CsvCellEdit): void {
  if (!Number.isSafeInteger(row) || !Number.isSafeInteger(column) || row < 0 || column < 0
    || (row + 1) * (column + 1) > CSV_MAX_CELLS) throw new Error("csv.tooLarge");
}

function assertCsvTextSize(text: string, byteLength = new TextEncoder().encode(text).byteLength): number {
  if (text.length > CSV_MAX_CHARACTERS || byteLength > CSV_MAX_CHARACTERS) throw new Error("csv.tooLarge");
  return byteLength;
}

function indexedCsvDocument(document: CsvDocument, records: CsvRecord[]): CsvCellEditResult {
  let offset = document.bom.length;
  const chunks = [document.bom];
  const indexed = records.map((record) => {
    const next = record.start === offset ? record : { ...record, start: offset };
    const serialized = next.fields.map((field) => field.raw).join(document.delimiter) + next.ending;
    chunks.push(serialized);
    offset += serialized.length;
    return next;
  });
  const text = chunks.join("");
  const byteLength = assertCsvTextSize(text);
  return { text, document: { ...document, records: indexed,
    columnCount: indexed.reduce((count, record) => Math.max(count, record.fields.length), 0), byteLength } };
}

export function csvDocumentChangedRows(previous: CsvDocument, current: CsvDocument): readonly number[] | null {
  const origin = csvEditOrigins.get(current);
  return origin?.previous === previous ? origin.rows : null;
}

function trackCsvCellEdit(result: CsvCellEditResult, previous: CsvDocument, edits: readonly CsvCellEdit[]): CsvCellEditResult {
  if (result.document !== previous) csvEditOrigins.set(result.document,
    { previous, rows: [...new Set(edits.map((edit) => edit.row))] });
  return result;
}

/**
 * Apply cell edits while keeping the parsed document usable. Existing cells are
 * patched directly into the source, so the common Enter-to-commit path does not
 * serialize and parse the complete CSV again.
 */
export function applyCsvCellEdits(document: CsvDocument, source: string, edits: readonly CsvCellEdit[]): CsvCellEditResult {
  const byCell = new Map<string, CsvCellEdit>();
  for (const edit of edits) { assertCsvCellEdit(edit); byCell.set(`${edit.row}:${edit.column}`, edit); }
  const changed = [...byCell.values()].filter(({ row, column, value }) =>
    value !== (document.records[row]?.fields[column]?.value ?? ""));
  if (!changed.length) return { document, text: source };

  const canPatchSource = changed.every(({ row, column }) => !!document.records[row]?.fields[column]);
  if (canPatchSource) {
    const records = document.records.slice();
    const copied = new Set<number>();
    const rowDeltas = new Map<number, number>();
    const patches: Array<{ start: number; end: number; text: string }> = [];
    let byteDelta = 0;
    for (const { row, column, value } of changed) {
      const originalRecord = document.records[row]!;
      const original = originalRecord.fields[column]!;
      const replacement = csvField(value, document.delimiter, original.raw.startsWith('"'));
      const start = originalRecord.start + originalRecord.fields.slice(0, column)
        .reduce((length, field) => length + field.raw.length + document.delimiter.length, 0);
      const end = start + original.raw.length;
      if (source.slice(start, end) !== original.raw) return trackCsvCellEdit(indexedCsvDocument(document,
        applyCsvEditsToRecords(document, changed)), document, changed);
      patches.push({ start, end, text: replacement.raw });
      const delta = replacement.raw.length - original.raw.length;
      rowDeltas.set(row, (rowDeltas.get(row) ?? 0) + delta);
      byteDelta += new TextEncoder().encode(replacement.raw).byteLength - new TextEncoder().encode(original.raw).byteLength;
      if (!copied.has(row)) {
        records[row] = { ...records[row]!, fields: records[row]!.fields.slice() };
        copied.add(row);
      }
      records[row]!.fields[column] = replacement;
    }
    patches.sort((left, right) => right.start - left.start);
    let text = source;
    for (const patch of patches) text = text.slice(0, patch.start) + patch.text + text.slice(patch.end);
    const byteLength = assertCsvTextSize(text, document.byteLength + byteDelta);
    let offsetDelta = 0;
    for (let index = 0; index < records.length; index++) {
      const start = document.records[index]!.start + offsetDelta;
      if (records[index]!.start !== start) records[index] = { ...records[index]!, start };
      offsetDelta += rowDeltas.get(index) ?? 0;
    }
    return trackCsvCellEdit({ text, document: { ...document, records, byteLength } }, document, changed);
  }

  return trackCsvCellEdit(indexedCsvDocument(document, applyCsvEditsToRecords(document, changed)), document, changed);
}

function applyCsvEditsToRecords(document: CsvDocument, edits: readonly CsvCellEdit[]): CsvRecord[] {
  const records = document.records.slice();
  const copied = new Set<number>();
  for (const { row, column, value } of edits) {
    if (value === (records[row]?.fields[column]?.value ?? "")) continue;
    while (records.length <= row) {
      const last = records[records.length - 1];
      if (last && !last.ending) records[records.length - 1] = { ...last, ending: document.newline };
      records.push({ fields: [], ending: "", start: 0 });
    }
    if (!copied.has(row)) {
      records[row] = { ...records[row]!, fields: records[row]!.fields.slice() };
      copied.add(row);
    }
    const fields = records[row]!.fields;
    while (fields.length <= column) fields.push(csvField("", document.delimiter));
    if (fields[column]!.value !== value) {
      fields[column] = csvField(value, document.delimiter, fields[column]!.raw.startsWith('"'));
    }
  }
  return records;
}

export function editCsvCells(document: CsvDocument, edits: readonly CsvCellEdit[]): string {
  return applyCsvCellEdits(document, serializeCsvDocument(document), edits).text;
}

export function insertCsvRow(document: CsvDocument, index: number): string {
  const records = document.records.slice();
  const at = Math.max(0, Math.min(records.length, index));
  const terminalNewline = !!records[records.length - 1]?.ending;
  if (at === records.length && records.length && !terminalNewline) {
    records[records.length - 1] = { ...records[records.length - 1]!, ending: document.newline };
  }
  records.splice(at, 0, {
    fields: Array.from({ length: Math.max(1, document.columnCount) }, () => csvField("", document.delimiter)),
    ending: at < records.length || terminalNewline ? document.newline : "", start: 0,
  });
  // One empty cell needs a record delimiter to distinguish it from an empty file.
  if (records.length === 1 && records[0]!.fields.length === 1) records[0]!.ending = document.newline;
  return serializeCsvDocument({ ...document, records });
}

export function deleteCsvRows(document: CsvDocument, rows: readonly number[]): string {
  const removed = new Set(rows);
  return serializeCsvDocument({ ...document, records: document.records.filter((_, index) => !removed.has(index)) });
}

export function changeCsvColumn(document: CsvDocument, index: number, remove: boolean): string {
  const at = Math.max(0, Math.min(document.columnCount, index));
  const records = document.records.map((row) => {
    const fields = row.fields.slice();
    if (remove) fields.splice(at, 1);
    else {
      while (fields.length < at) fields.push(csvField("", document.delimiter));
      fields.splice(at, 0, csvField("", document.delimiter));
    }
    return { ...row, fields: fields.length ? fields : [csvField("", document.delimiter)] };
  });
  return serializeCsvDocument({ ...document, records });
}

export function parseCsvClipboard(text: string): string[][] {
  const parsed = Papa.parse<string[]>(text, { delimiter: "\t", dynamicTyping: false, skipEmptyLines: false });
  if (parsed.errors.length) throw new Error("csv.invalidClipboard");
  const rows = parsed.data;
  if (/[\r\n]$/.test(text) && rows[rows.length - 1]?.length === 1 && rows[rows.length - 1]?.[0] === "") rows.pop();
  if (rows.reduce((total, row) => total + row.length, 0) > CSV_MAX_CELLS) throw new Error("csv.tooLarge");
  return rows;
}

export function serializeCsvClipboard(rows: string[][]): string {
  return Papa.unparse(rows, { delimiter: "\t", newline: "\n", skipEmptyLines: false });
}
