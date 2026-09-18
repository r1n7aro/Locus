import { describe, expect, it } from "vitest";
import { parseCsvDocument } from "../document/csv/csvDocument";
import { defaultCsvView, parseCsvView, reconcileCsvView, serializeCsvView } from "../document/csv/csvView";
import { adjustCsvMerges, csvMergeEdits, mergeCsvSelection, migrateCsvViewToV3, unmergeCsvSelection, validateCsvMerges, type CsvMerge } from "../document/csv/csvMerges";
import { migrateCsvViewToV2 } from "../document/csv/csvStyles";
import { projectCsvColumns, projectCsvRowIndices } from "../document/csv/csvGridProjection";
import { projectCsvMerges } from "../components/csv/csvMergeLayer";

const rectangle: CsvMerge = { rows: [1, 2], columns: [0, 1] };
describe("CSV merged ranges", () => {
  it("migrates old views explicitly, preserves formatting, and round-trips compact rectangles", () => {
    for (const schema of ["locus.csv-view.v1", "locus.csv-view.v2"] as const) {
      const old = { ...defaultCsvView(), schema };
      const migrated = mergeCsvSelection(old, rectangle);
      expect(old.schema).toBe(schema);
      expect(migrateCsvViewToV3(migrated)).toEqual(migrated);
      expect(migrateCsvViewToV2(migrated)).toEqual(migrated);
      expect(parseCsvView(serializeCsvView(migrated))).toEqual(migrated);
      expect(serializeCsvView(migrated)).toContain('merges:\n  - {"rows":[1,2],"columns":[0,1]}\n');
      expect(() => parseCsvView(serializeCsvView({ ...old, merges: [rectangle] }))).toThrow();
    }
  });
  it("rejects overlaps, invalid coordinates and unbounded metadata", () => {
    for (const value of [null, {}, [null], [{ rows: [0, 0], columns: [0, 0] }],
      [{ rows: [2, 1], columns: [0, 1] }], [{ rows: [-1, 1], columns: [0, 1] }],
      [{ rows: [0, 1.5], columns: [0, 1] }], [{ rows: [0, 1], columns: [0, 10000] }],
      [{ rows: [0, 499999], columns: [0, 1] }], [rectangle, rectangle],
      [rectangle, { rows: [2, 4], columns: [1, 2] }], [{ ...rectangle, extra: true }]]) {
      expect(validateCsvMerges(value), JSON.stringify(value)).toBe(false);
    }
    expect(validateCsvMerges([rectangle, { rows: [1, 2], columns: [2, 3] }])).toBe(true);
    expect(validateCsvMerges([{ rows: [0, 499999], columns: [0, 0] }])).toBe(true);
  });
  it("expands partial intersections and unmerges only selected ranges without changing data", () => {
    const original = mergeCsvSelection(defaultCsvView(), rectangle);
    const extended = mergeCsvSelection(original, { rows: [2, 3], columns: [1, 2] });
    expect(extended.merges).toEqual([{ rows: [1, 3], columns: [0, 2] }]);
    expect(original.merges).toEqual([rectangle]);
    expect(unmergeCsvSelection(extended, [0], [0]).merges).toEqual(extended.merges);
    expect(unmergeCsvSelection(extended, [2], [1]).merges).toEqual([]);
  });
  it("follows source row/column insertions, deletions, anchor removal and complete removal", () => {
    expect(adjustCsvMerges([rectangle], "rows", 0)).toEqual([{ rows: [2, 3], columns: [0, 1] }]);
    expect(adjustCsvMerges([rectangle], "rows", 2)).toEqual([{ rows: [1, 3], columns: [0, 1] }]);
    expect(adjustCsvMerges([rectangle], "columns", 1)).toEqual([{ rows: [1, 2], columns: [0, 2] }]);
    expect(adjustCsvMerges([rectangle], "rows", null, [1])).toEqual([{ rows: [1, 1], columns: [0, 1] }]);
    expect(adjustCsvMerges([rectangle], "rows", null, [1, 2])).toEqual([]);
    expect(adjustCsvMerges([{ rows: [1, 1], columns: [0, 1] }], "columns", null, [0])).toEqual([]);
  });
  it("edits each visible anchor once and preserves covered values", () => {
    const edits = [{ row: 1, column: 0, value: "x" }, { row: 1, column: 1, value: "" }, { row: 2, column: 0, value: "" }, { row: 3, column: 2, value: "y" }];
    expect(csvMergeEdits([rectangle], edits)).toEqual([edits[0], edits[3]]);
    expect(csvMergeEdits([rectangle], [{ row: 2, column: 1, value: "filled" }], true)).toEqual([{ row: 1, column: 0, value: "filled" }]);
  });
  it("sorts and filters row groups without separating merged records", () => {
    const doc = parseCsvDocument("name,note\nz,first\na,match\nm,other\n");
    const view = mergeCsvSelection(reconcileCsvView(defaultCsvView(), doc), rectangle);
    const columnId = view.columnOrder[0]!;
    view.sort = [{ columnId, direction: "asc" }];
    expect(projectCsvRowIndices(doc, view, 26).slice(0, 4)).toEqual([0, 3, 1, 2]);
    view.filters = [{ columnId: view.columnOrder[1]!, value: "match" }];
    expect(projectCsvRowIndices(doc, view, 26).slice(0, 4)).toEqual([0, 1, 2, 4]);
  });
  it("keeps moved columns grouped, shrinks hidden spans and restores blank ranges", () => {
    const doc = parseCsvDocument("a,b,c,d\n1,2,3,4\n");
    const view = mergeCsvSelection(reconcileCsvView(defaultCsvView(), doc), rectangle);
    const [a, b, c, d] = view.columnOrder as [string, string, string, string];
    view.columnOrder = [d, b, c, a];
    let columns = projectCsvColumns(doc, view);
    expect(columns.slice(0, 4).map((column) => column.sourceIndex)).toEqual([3, 0, 1, 2]);
    view.columns[a]!.hidden = true;
    columns = projectCsvColumns(doc, view).filter((column) => !column.hidden);
    expect(projectCsvMerges(view.merges!, projectCsvRowIndices(doc, view, columns.length), columns)[0]).toMatchObject({ columns: [1, 1], source: rectangle });
    const blank = mergeCsvSelection(defaultCsvView(), { rows: [100, 110], columns: [29, 30] });
    const blankDoc = parseCsvDocument("");
    const blankColumns = projectCsvColumns(blankDoc, blank), blankRows = projectCsvRowIndices(blankDoc, blank, 31);
    expect(blankColumns[blankColumns.length - 1]!.sourceIndex).toBe(30);
    expect(blankRows[blankRows.length - 1]).toBe(110);
  });
});
