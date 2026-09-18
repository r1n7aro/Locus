"""CSV worksheet editing with openpyxl and a low-level, versioned view API."""
from __future__ import annotations

import copy
import os
from dataclasses import dataclass, field
from typing import Any, TYPE_CHECKING

from ._scope import workspace_payload

if TYPE_CHECKING:
    from ._client import Client


@dataclass(frozen=True)
class CsvColumnView:
    source_index: int
    header: str
    width: int
    hidden: bool = False


@dataclass(frozen=True)
class CsvViewSnapshot:
    """Current layout and an opaque revision of both the CSV and its view file.

    ``row_count`` includes the source header row. ``columns`` maps persistent
    column IDs to source bindings; ``column_order`` describes display order.
    Reading defaults never creates a view file.
    """

    file_path: str
    revision: str
    row_count: int
    column_count: int
    delimiter: str
    schema: str
    header_rows: int
    row_height: int
    wrap_text: bool
    frozen_columns: int
    columns: dict[str, CsvColumnView]
    column_order: list[str]
    sort: list[dict[str, str]]
    filters: list[dict[str, str]]
    styles: list[dict[str, Any]]
    merges: list[dict[str, list[int]]] = field(default_factory=list)
    row_dimensions: dict[str, dict[str, Any]] = field(default_factory=dict)

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> CsvViewSnapshot:
        view = payload["view"]
        return cls(
            file_path=payload["filePath"], revision=payload["revision"],
            row_count=payload["rowCount"], column_count=payload["columnCount"],
            delimiter=payload["delimiter"], schema=view["schema"], header_rows=view["headerRows"],
            row_height=view["rowHeight"], wrap_text=view["wrapText"],
            frozen_columns=view["frozenColumns"],
            columns={key: CsvColumnView(value["sourceIndex"], value["header"],
                value["width"], value.get("hidden", False)) for key, value in view["columns"].items()},
            column_order=list(view["columnOrder"]),
            sort=[{"column_id": item["columnId"], "direction": item["direction"]} for item in view.get("sort", [])],
            filters=[{"column_id": item["columnId"], "value": item["value"]} for item in view.get("filters", [])],
            styles=copy.deepcopy(view.get("styles", [])),
            merges=copy.deepcopy(view.get("merges", [])),
            row_dimensions=copy.deepcopy(view.get("rowDimensions", {})),
        )


class Csv:
    def __init__(self, client: Client):
        self._client = client

    @staticmethod
    def _path(file_path: str) -> str:
        if not isinstance(file_path, str) or not file_path.strip().lower().endswith(".csv"):
            raise ValueError("file_path must name the CSV document, not its .view companion")
        return file_path.strip()

    async def load_workbook(self, filename: str, *, workspace_ref: Any = None, worktree: Any = None):
        """Load one CSV as an openpyxl workbook. Call ``await wb.save()`` to persist.

        All values initially remain strings. Formatting goes to the adjacent
        .csv.view. No Excel protection, passwords, sheets or calculation engine.
        """
        try:
            from ._csv_workbook import CsvWorkbook
        except ModuleNotFoundError as error:
            if error.name not in {"openpyxl", "et_xmlfile"}: raise
            raise RuntimeError("CSV workbook editing requires openpyxl: python -m pip install openpyxl==3.1.5") from error
        scope = workspace_payload(workspace_ref, worktree)
        result = await self._client.rpc("csv.read_workbook", {
            "workspaceRef": scope, "filePath": self._path(os.fspath(filename)),
        })
        return CsvWorkbook(self._client, result, scope)

    async def read_view(self, file_path: str, *, workspace_ref: Any = None,
                        worktree: Any = None) -> CsvViewSnapshot:
        """Read the current layout and revision; no open editor is required."""
        result = await self._client.rpc("csv.read_view", {
            "workspaceRef": workspace_payload(workspace_ref, worktree),
            "filePath": self._path(file_path),
        })
        return CsvViewSnapshot.from_payload(result)

    async def patch_view(self, file_path: str, patch: dict[str, Any], *,
                         expected_revision: str, workspace_ref: Any = None,
                         worktree: Any = None) -> CsvViewSnapshot:
        """Apply a layout batch against a read revision; cell data is preserved.

        Patch fields use snake_case: header_rows, row_height, wrap_text,
        frozen_columns, columns, column_order, sort, filters, styles, merges. Column entries
        contain target (exactly one of id, source_index, header), width and/or
        hidden. Sort/filter entries contain target and direction/value.
        column_order is a complete list of targets. Omitted fields are kept;
        empty sort/filters lists clear them. Duplicate headers require an ID
        or zero-based source_index. Re-read after a revision conflict.
        styles accepts upsert/remove/order for sparse rules with an id, optional
        rows range, columns selectors, when condition and a style. Styles support
        font, size, bold, color, background and border. Later rules win.
        merges replaces all merged ranges; [] unmerges everything. Each range has
        rows: [start, end] and columns: [start, end], inclusive zero-based source
        coordinates. Only the top-left value is displayed; all data is retained.
        """
        if not isinstance(expected_revision, str) or not expected_revision.strip():
            raise ValueError("expected_revision must come from read_view() or a previous successful patch_view()")
        if not isinstance(patch, dict) or not patch:
            raise ValueError("patch must be a non-empty dictionary")
        result = await self._client.rpc("csv.patch_view", {
            "workspaceRef": workspace_payload(workspace_ref, worktree),
            "filePath": self._path(file_path), "patch": copy.deepcopy(patch),
            "expectedRevision": expected_revision,
            "executionDelegation": os.environ.get("LOCUS_SDK_EXECUTION_DELEGATION"),
            "sessionId": self._client.current_session_id,
        })
        return CsvViewSnapshot.from_payload(result)
