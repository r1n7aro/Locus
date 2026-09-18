import { describe, expect, it } from "vitest";
import { applyCsvCellEdits, changeCsvColumn, csvDocumentChangedRows, deleteCsvRows, editCsvCells, insertCsvRow, parseCsvClipboard,
  parseCsvDocument, serializeCsvClipboard, serializeCsvDocument } from "../document/csv/csvDocument";
import { defaultCsvView, parseCsvView, reconcileCsvView, serializeCsvView } from "../document/csv/csvView";

describe("CSV lexical document", () => {
  it.each(["", "\ufeff", "a,b,\r\n001,\"hello, world\",\r\n\r\n", 'a,b\n"a\r\nb","x""y"\n',
    "a;b\r1;2\r", "first\tsecond\n001\t  \n", "a,b\n1\n2,3,4", "\n\n", "a,b\n1,2"]) (
    "round trips %j byte-for-byte as UTF-8 text", (text) => expect(serializeCsvDocument(parseCsvDocument(text))).toBe(text),
  );
  it("edits only the changed field while retaining BOM, unnecessary quotes and embedded CRLF", () => {
    const source = '\ufeff"id",name,note,\r\n001,"name","a\r\nb",\r\n\r\n';
    const output = editCsvCells(parseCsvDocument(source), [{ row: 1, column: 1, value: '新,"值"' }]);
    expect(output).toBe('\ufeff"id",name,note,\r\n001,"新,""值""","a\r\nb",\r\n\r\n');
    expect(parseCsvDocument(output).records[1]!.fields[0]!.value).toBe("001");
  });
  it("preserves ragged rows until an explicit edit extends one", () => {
    const source = "id,a,b\n001\n002,x,y\n";
    expect(editCsvCells(parseCsvDocument(source), [{ row: 1, column: 2, value: "z" }]))
      .toBe("id,a,b\n001,,z\n002,x,y\n");
  });
  it("patches existing cells without reparsing or replacing unaffected field arrays", () => {
    const source = "id,name,note\r\n001,first,keep\r\n002,second,tail\r\n";
    const original = parseCsvDocument(source);
    const first = applyCsvCellEdits(original, source, [{ row: 1, column: 1, value: "a longer value" }]);
    expect(first.text).toBe("id,name,note\r\n001,a longer value,keep\r\n002,second,tail\r\n");
    expect(first.document.records[0]!.fields).toBe(original.records[0]!.fields);
    expect(first.document.records[1]!.fields).not.toBe(original.records[1]!.fields);
    expect(first.document.records[2]!.fields).toBe(original.records[2]!.fields);
    expect(first.document.records[2]!.start).toBe(original.records[2]!.start + 9);
    expect(csvDocumentChangedRows(original, first.document)).toEqual([1]);

    const second = applyCsvCellEdits(first.document, first.text, [{ row: 2, column: 1, value: "final" }]);
    expect(second.text).toBe("id,name,note\r\n001,a longer value,keep\r\n002,final,tail\r\n");
    expect(serializeCsvDocument(second.document)).toBe(second.text);
  });
  it.each(['"unclosed', 'a,b\n"closed"oops,2', 'a,un"quoted']) ("rejects malformed quoting %j", (text) => {
    expect(() => parseCsvDocument(text)).toThrow();
  });
  it("handles insertion/deletion without trimming cells or requiring a final newline", () => {
    const source = "id,a\r\n001,  ";
    expect(insertCsvRow(parseCsvDocument(source), 1)).toBe("id,a\r\n,\r\n001,  ");
    expect(deleteCsvRows(parseCsvDocument(source), [1])).toBe("id,a\r\n");
    expect(changeCsvColumn(parseCsvDocument(source), 1, false)).toBe("id,,a\r\n001,,  ");
    expect(changeCsvColumn(parseCsvDocument(source), 1, true)).toBe("id\r\n001");
    expect(insertCsvRow(parseCsvDocument(""), 0)).toBe("\n");
  });
  it("copies multiline cells and identifiers as text and removes only the clipboard terminator", () => {
    const rows = [["001", "a\nb", "=SUM(A1)", ""], ["  ", "x\ty", 'a"b', ""]];
    expect(parseCsvClipboard(serializeCsvClipboard(rows))).toEqual(rows);
    expect(parseCsvClipboard("a\tb\r\n")).toEqual([["a", "b"]]);
  });
});

describe("YAML CSV view", () => {
  const view = () => reconcileCsvView(defaultCsvView(), parseCsvDocument("id,name\n001,first\n"));
  it("round trips deterministic mappings and keeps display ordering separate", () => {
    const original = view();
    const text = serializeCsvView(original);
    expect(serializeCsvView(parseCsvView(text))).toBe(text);
    const reversed = { ...original, columnOrder: original.columnOrder.slice().reverse() };
    expect(serializeCsvView(reversed).split("columnOrder:")[0]).toBe(text.split("columnOrder:")[0]);
  });
  it("rejects duplicate keys, aliases, unknown schema and merge conflicts", () => {
    const text = serializeCsvView(view());
    expect(() => parseCsvView(`${text}rowHeight: 40\n`)).toThrow();
    expect(() => parseCsvView(text.replace("locus.csv-view.v1", "locus.csv-view.v999"))).toThrow();
    expect(() => parseCsvView("<<<<<<< HEAD\n" + text)).toThrow();
    expect(() => parseCsvView(`${text}extra: &alias [x]\nother: *alias\n`)).toThrow();
  });
  it("rebinds distinct externally reordered headers while retaining column IDs and widths", () => {
    const original = view();
    const id = original.columnOrder[0]!;
    original.columns[id]!.width = 200;
    const reordered = reconcileCsvView(original, parseCsvDocument("name,id\nfirst,001\n"));
    expect(reordered.columns[id]).toMatchObject({ sourceIndex: 1, header: "id", width: 200 });
  });
  it("accepts duplicate/empty headers as distinct columns", () => {
    const result = reconcileCsvView(defaultCsvView(), parseCsvDocument("id,id,\n1,2,3\n"));
    expect(result.columnOrder).toHaveLength(3);
    expect(new Set(result.columnOrder).size).toBe(3);
    expect(parseCsvView(serializeCsvView(result)).columnOrder).toEqual(result.columnOrder);
  });
});
