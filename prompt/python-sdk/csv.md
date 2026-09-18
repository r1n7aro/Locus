# CSV tables with openpyxl

Edit Locus CSV tables with the **same worksheet and style objects as openpyxl**, the widely used Python Excel-editing library. `load_workbook` returns a real openpyxl `Workbook` with one worksheet. Reuse your openpyxl knowledge; the Locus-specific steps are asynchronous loading and saving to the original CSV.

```python
from openpyxl.styles import Font, PatternFill, Border, Side, Alignment, NamedStyle
from openpyxl.formatting.rule import CellIsRule

# Run edits with python readonly=false.
wb = await locus.csv.load_workbook("Locus/knowledge/design/items.csv")
ws = wb.active

for row in ws["A1:D1"]:
    for cell in row:
        cell.font = Font(name="Arial", size=12, bold=True, color="FFFFFF")
        cell.fill = PatternFill("solid", fgColor="334155")
        cell.alignment = Alignment(horizontal="center", vertical="center", wrap_text=True)
        cell.border = Border(bottom=Side(style="thin", color="64748B"))

ws["B2"] = "Updated label"
ws["C2"].number_format = "#,##0.00"
ws["D2"].number_format = "0.0%"
ws.column_dimensions["B"].width = 28
ws.row_dimensions[1].height = 30
ws.conditional_formatting.add("C2:C100", CellIsRule(
    operator="greaterThan", formula=[100], fill=PatternFill("solid", fgColor="FFF2CC")))
await wb.save()
```

Use `ws["A1"]`, `ws.cell(row=1, column=1)`, `ws["A1:C3"]`, `iter_rows`, `iter_cols`, `append`, `copy.copy(cell.font)`, and `NamedStyle` normally. A1 / 1-based coordinates always address source CSV records, including the header; display sorting/filtering never changes those addresses. `load_workbook` accepts `workspace_ref=` or `worktree=`. `save()` keeps the loaded revision internally and batches changes; a conflict raises without retrying or discarding pending edits. Reload and reconsider the edit after a conflict.

CSV remains text. Loaded values are strings (including `001`, dates and text beginning with `=`). Assign strings, finite numbers, booleans, `None` (empty), or Python date/time values (ISO text). Formulas are stored as text and never calculated. Merely applying a number format changes display, not source values. Styling alone preserves CSV bytes; value edits replace only changed fields, retaining other quoting, BOM, delimiters and record endings. `merge_cells` / `unmerge_cells` retain covered source values, unlike Excel's destructive merging.

Supported display features:

- `Font`: installed font family, point size (including fractional), bold, italic, underline, strike, color and super/subscript.
- `PatternFill`: solid and pattern fills. Pattern geometry is approximated by CSS.
- `Border` / `Side`: independent top/right/bottom/left edges; thin/medium/thick/hair, dashed/dotted/double. Dash-dot variants approximate a dashed line.
- `Alignment`: left/center/right/general/justify/distributed, top/center/bottom, wrap, indent, reading order, rotation/vertical text and shrink-to-fit. Justified/distributed text and super/subscript placement use browser typography rather than Excel's layout engine.
- `number_format`: standard Excel format strings rendered through SheetJS SSF (decimal, grouping, percentages, currency, scientific, fractions and dates). General / `@` retain raw text; explicit numeric formats can format numeric CSV strings. Format-section colors and full Excel locale behavior are not implemented.
- `NamedStyle` and style copying; styles are persisted by value, not as an Excel theme/style catalog.
- Column widths (Excel character units, converted with 7px per character + 5px padding), hidden columns; per-row heights (points), hidden rows; default row height; `freeze_panes="C1"` for leading columns.
- `CellIsRule` with one literal comparison and no `stopIfTrue`. Conditional font overrides currently cover name, size, bold and color; italic/underline/strike remain direct-format features. Existing Locus conditions remain dynamic and are retained.

Unsupported display features raise on save: gradient fills, diagonal/inside borders, outline/shadow/condensed/extended fonts, `fill` / `centerContinuous` alignment, formula-based conditional formatting, between/notBetween, color scales, data bars, icon sets and `stopIfTrue`. Freezing rows and row/column insertion/deletion/moving ranges are not exposed by this adapter; use the table editor for structural operations. No workbook passwords, sheet/cell protection, multi-sheet workbook, printing, Excel tables, charts, images, validation, comments, hyperlinks or formula calculation are added to CSV. Avoid these Excel-only settings. Do not call `openpyxl.load_workbook` or synchronous `Workbook.save` on CSV files.

RGB / aRGB, indexed and standard Office theme colors are accepted; theme/indexed colors resolve to RGB at save. Excel ignores the alpha part of aRGB style colors. Font availability and browser text metrics can differ from Excel. Large repeated style edits are compacted into row/range rules before persistence. Limits remain 16 MiB CSV, 500,000 rectangular cells, 10,000 columns, 10,000 style rules and 1 MiB view file.

The managed Python build includes openpyxl 3.1.5. With an external Python runtime, install it using `python -m pip install openpyxl==3.1.5` if needed. Other Locus SDK APIs do not require this dependency.

## Low-level layout and sparse rule API

Use `locus.csv` to read and edit the persisted layout of a CSV document. The editor and SDK share validation and version-checked persistence. The table does not need to be open. Pass its checkout-relative `.csv` path, never the `.csv.view` path.

```python
path = "Locus/knowledge/design/动画需求表.csv"
view = await locus.csv.read_view(path)
print(view.revision, view.row_count, view.column_count)
print([(key, col.source_index, col.header, col.width) for key, col in view.columns.items()])

# Run this mutation with python readonly=false.
updated = await locus.csv.patch_view(path, {
    "wrap_text": True,
    "row_height": 40,
    "frozen_columns": 2,
    "columns": [
        {"target": {"header": "建议Clip名"}, "width": 240},
        {"target": {"header": "表现与节奏要求"}, "width": 360},
    ],
}, expected_revision=view.revision)
print(updated.revision)
```

Both methods accept `workspace_ref=reference` or `worktree=handle`. Omission uses the Python tool's injected checkout. The snapshot's `file_path` is checkout-relative and can be passed back to `patch_view`.

`read_view` returns a `CsvViewSnapshot` with `revision`, `schema`, `row_count` (including the source header), `column_count`, `delimiter`, `header_rows`, `row_height`, `wrap_text`, `frozen_columns`, `columns`, `column_order`, `sort`, `filters`, and `styles`. `columns` maps column IDs to `CsvColumnView` values (`source_index`, `header`, `width`, `hidden`). `column_count` counts source columns; the view can also contain configured blank worksheet columns. Reading a missing companion returns defaults without creating a file.

Patch fields are optional; omitted fields retain their current values:

| Field | Value |
| --- | --- |
| `header_rows` | `0` or `1`; interpret the first source record as a header |
| `row_height` | Integer pixels, `20..120` |
| `wrap_text` | Boolean, applies to the whole table |
| `frozen_columns` | Number of leading visible columns to freeze; at most the configured column count |
| `columns` | List of `{ "target": selector, "width": 48..2000, "hidden": bool }`; provide width and/or hidden |
| `column_order` | Complete list of column selectors, each configured column exactly once |
| `sort` | List of `{ "target": selector, "direction": "asc" or "desc" }`; `[]` clears sorting |
| `filters` | List of `{ "target": selector, "value": "text" }`; `[]` clears filtering |
| `styles` | `{ "upsert": [rules], "remove": [rule_ids], "order": [all_remaining_rule_ids] }`; each field is optional |

A selector contains exactly one of `{"id": column_id}`, `{"source_index": 0}`, or `{"header": "unique header"}`. Source indices are zero-based and do not change when display columns move. Duplicate or empty headers may be ambiguous: use a returned ID or source index. Every selector in a batch resolves against the original snapshot, including when the batch changes `header_rows`. Column patches must target distinct columns. Snapshot `sort`/`filters` use `column_id`; when writing, wrap it as `target={"id": ...}`.

Sorting is lexical and affects display only. Filters use case-insensitive substring matching, combined with AND. These operations preserve CSV content, delimiter, quoting, BOM and line endings byte-for-byte. Changes persist to the adjacent YAML `.csv.view`; `.meta` files are untouched. A no-op does not create or rewrite the companion.

Always pass `expected_revision` from a read or successful patch. It covers both files and the document identity. After a conflict, read the current state and reconsider the patch; do not silently retry with a refreshed revision. Invalid or ambiguous batches write nothing. Knowledge source permissions and AI edit policy apply to the owning CSV. Patching is unavailable in Plan mode.

Open clean editors refresh after writes. Unsaved local edits remain preserved and may require conflict resolution. SDK changes do not currently enter the editor's Ctrl+Z history. Use file tools for authorized CSV cell data edits; use this SDK for layouts and formatting instead of writing `.csv.view` as a knowledge document.

## Sparse formatting rules

```python
view = await locus.csv.read_view(path)
updated = await locus.csv.patch_view(path, {"styles": {"upsert": [
    {"id": "header", "rows": [0, 0], "style": {
        "font": "sans", "size": 15, "bold": True, "background": "subtle",
        "border": {"color": "border", "width": 1, "edges": ["bottom"]},
    }},
    {"id": "pending", "when": {
        "target": {"header": "新增Clip估算"}, "op": "contains", "value": "待确认",
    }, "style": {"color": "warning", "background": "warning-soft"}},
    {"id": "clip-exception", "rows": [17, 17],
     "columns": [{"header": "建议Clip名"}],
     "style": {"font": "mono", "size": 14, "bold": True, "color": "#7c9bdb"}},
]}}, expected_revision=view.revision)
```

Each rule has a stable `id`, a non-empty `style`, and optional `rows`, `columns` and `when`. Omit both rows and columns to format the whole table. `rows: [start, end]` is an inclusive range of zero-based **source record** indices (row 0 includes the CSV header); omit columns for a whole-row rule. A single cell uses `[row, row]` plus one column selector. A rectangle uses one row range plus selected columns. Whole rows and ranges remain single rules on disk: never emit a style record for every matching cell.

`upsert` replaces the complete rule with that ID in its existing position, or appends a new rule. Omit unwanted fields when replacing a rule to clear those overrides. `remove` deletes existing IDs. `order`, when provided, must list every remaining rule exactly once. Later matching rules override only their specified properties; borders override the specified edges. Use a late cell rule for exceptions to a row/conditional rule. Repeated identical upserts do not grow the file.

| Style field | Value |
| --- | --- |
| `font` | `"ui"`, `"sans"`, `"mono"`, or an installed font family name such as `"Arial"`; unavailable fonts fall back to the UI font |
| `size` | Integer pixels, `8..72` |
| `bold` | Boolean; `False` overrides an earlier bold rule |
| `color`, `background` | Hex `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`, or a semantic color below |
| `border` | Optional `color`, `width` (`0..4` pixels), `style` (`solid`, `dashed`, `dotted`, `double`, `none`) and `edges` (subset of `top`, `right`, `bottom`, `left`). Defaults: border token, 1px solid, all edges |

Semantic colors: `text`, `secondary`, `accent`, `success`, `warning`, `error`, `surface`, `subtle`, `accent-soft`, `success-soft`, `warning-soft`, `error-soft`, `border`, `transparent`. `default` restores the normal color for that property. Prefer semantic colors for theme compatibility.

A `when` condition contains a column `target`, an `op`, and normally a `value` (string or number). Text operators `eq`, `ne`, `contains`, `not_contains` are case-sensitive. Numeric operators `gt`, `gte`, `lt`, `lte` compare finite numbers; empty or nonnumeric cells do not match. `empty` and `not_empty` take no value and trim whitespace. Conditions are evaluated from the source row's current cell values and can format the entire row or just the rule's selected columns. Snapshot `styles` contain persisted column IDs (`columns: [id]`, `when.columnId`); use column selectors when constructing upserts.

Sorting/filtering does not change the source row addresses. Inserting/deleting rows through the table editor adjusts stored row ranges. For identities that must survive external CSV row reordering, use a condition on a unique key such as `需求编号`, rather than a fixed row index.

Legacy formatting explicitly upgrades v1 to `locus.csv-view.v2`; merged ranges use v3; openpyxl styles and per-row dimensions use v4. Reads leave old files untouched. Each migration is idempotent, preserves earlier fields, and cannot silently downgrade a newer file. Styles persist as compact ordered rule mappings; matching results are computed at render time. Prefer the workbook API for new scripts; this lower-level API remains available for Locus-specific sparse conditional rules, display sorting and filtering.
