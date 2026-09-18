import type { ColumnComponent, Tabulator } from "tabulator-tables";

// Tabulator 6.5's range model is usable with frozen columns, but its overlay
// indexes the horizontal renderer's non-frozen window as if it contained every
// column. Keep the model/events and replace the geometry on this instance only.
// Install before Tabulator's deferred build binds the module's event handlers.
interface RangeColumn {
  definition: { frozen?: boolean; headerSort?: boolean; editor?: unknown; cssClass?: string };
  getWidth(): number;
  getElement(): HTMLElement;
}
interface RangeRow { position: number; type: string; getElement(): HTMLElement }
interface RangeBounds { top: number; bottom: number; left: number; right: number }
interface InternalRange extends RangeBounds { element: HTMLElement; start: { row: number; col: number }; end: { row: number; col: number } }
interface RangeModule {
  table: { initialized: boolean; rtl: boolean; element: HTMLElement; rowManager: { element: HTMLElement; getVisibleRows(chain: boolean): RangeRow[] } };
  rowSelection: boolean;
  rowHeader: RangeColumn | null;
  ranges: InternalRange[];
  activeRange: InternalRange | false;
  overlay: HTMLElement;
  activeRangeCellElement: HTMLElement;
  getTableColumns(): RangeColumn[];
  getRowByRangePos(index: number): RangeRow | undefined;
  updateHeaderColumn(): void;
  layoutRanges(): void;
  layoutChange(): void;
  autoScroll(range: InternalRange): void;
  mouseUpEvent(): void;
}

function columnGeometry(columns: readonly { getWidth(): number }[], frozen: (index: number) => boolean) {
  const edges = [0];
  let frozenCount = 0;
  for (const [index, column] of columns.entries()) {
    edges.push(edges[index]! + column.getWidth());
    if (index === frozenCount && frozen(index)) frozenCount++;
  }
  return { edges, frozenCount, boundary: edges[frozenCount]! };
}

function scrollColumn(scroller: HTMLElement, index: number, geometry: ReturnType<typeof columnGeometry>): boolean {
  if (index < geometry.frozenCount || index < 0 || geometry.boundary >= scroller.clientWidth) return false;
  const left = geometry.edges[index]!, right = geometry.edges[index + 1]!;
  const previous = scroller.scrollLeft;
  if (left < previous + geometry.boundary) scroller.scrollLeft = left - geometry.boundary;
  else if (right > previous + scroller.clientWidth) scroller.scrollLeft = Math.min(left - geometry.boundary, right - scroller.clientWidth);
  return previous !== scroller.scrollLeft;
}

/** Reveal an ordinary column beyond the whole frozen prefix; never scroll a frozen cell. */
export function scrollCsvColumnIntoView(table: Tabulator, target: ColumnComponent): boolean {
  const scroller = table.element.querySelector<HTMLElement>(".tabulator-tableholder");
  if (!scroller) return false;
  const columns = table.getColumns().filter((column) => column.isVisible());
  return scrollColumn(scroller, columns.indexOf(target), columnGeometry(columns, (index) => !!columns[index]!.getDefinition().frozen));
}

export function installCsvRangeSelection(table: Tabulator): void {
  const module = (table as unknown as { modules?: { selectRange?: RangeModule } }).modules?.selectRange;
  if (!module) return;
  const originalHeader = module.updateHeaderColumn;
  const originalLayout = module.layoutRanges;
  const originalScroll = module.autoScroll;
  const fragments = new WeakMap<InternalRange, HTMLElement[]>();
  let supported = true;
  let frame: { window: Window; id: number } | null = null;
  let destroyed = false;
  let gestureDocument: Document | null = null;
  const releaseGesture = () => {
    gestureDocument?.removeEventListener("mouseup", releaseGesture);
    gestureDocument?.defaultView?.removeEventListener("blur", releaseGesture);
    gestureDocument = null;
    module.mouseUpEvent();
  };
  const beginGesture = () => {
    releaseGesture();
    // Upstream listens on the creating document. A shared workbench popup
    // adopts this grid's DOM, so mouseup must follow its current owner document.
    gestureDocument = table.element.ownerDocument;
    gestureDocument.addEventListener("mouseup", releaseGesture);
    gestureDocument.defaultView?.addEventListener("blur", releaseGesture);
  };
  table.element.addEventListener("mousedown", beginGesture, true);

  module.updateHeaderColumn = function () {
    const columns = this.getTableColumns();
    const geometry = columnGeometry(columns, (index) => !!columns[index]!.definition.frozen);
    const header = this.rowSelection ? columns[0] : undefined;
    // Locus uses a flat LTR grid with a contiguous frozen prefix. Preserve the
    // upstream diagnostics if a future configuration falls outside that scope.
    supported = !this.table.rtl;
    supported &&= columns.every((column, index) => !column.definition.frozen || index < geometry.frozenCount)
      && !header?.definition.headerSort && !header?.definition.editor;
    if (!supported) { originalHeader.call(this); return; }
    this.rowHeader = header ?? null;
    if (header && !header.definition.cssClass?.split(/\s+/).includes("tabulator-range-row-header")) {
      header.definition.cssClass = `${header.definition.cssClass ?? ""} tabulator-range-row-header`.trim();
    }
  };

  module.layoutRanges = function () {
    if (destroyed || !this.table.initialized || !this.overlay) return;
    if (!supported) { originalLayout.call(this); return; }
    const scroller = this.table.rowManager.element;
    const columns = this.getTableColumns();
    const { edges, frozenCount, boundary } = columnGeometry(columns, (index) => !!columns[index]!.definition.frozen);
    const viewport = scroller.getBoundingClientRect();
    const rows = this.table.rowManager.getVisibleRows(true).filter((row) => row.type === "row" && row.getElement().isConnected);
    const reversedRows = [...rows].reverse();
    Object.assign(this.overlay.style, { left: `${scroller.scrollLeft}px`, top: `${scroller.scrollTop}px`,
      width: `${scroller.clientWidth}px`, height: `${scroller.clientHeight}px`, overflow: "hidden", visibility: "visible" });

    const draw = (element: HTMLElement, bounds: RangeBounds, fixed: boolean) => {
      element.style.display = "none";
      const leftColumn = Math.max(bounds.left, fixed ? 0 : frozenCount);
      const rightColumn = Math.min(bounds.right, fixed ? frozenCount - 1 : columns.length - 1);
      const first = rows.find((row) => row.position - 1 >= bounds.top && row.position - 1 <= bounds.bottom);
      const last = reversedRows.find((row) => row.position - 1 >= bounds.top && row.position - 1 <= bounds.bottom);
      if (leftColumn > rightColumn || !first || !last) return;
      const left = edges[leftColumn]! - (fixed ? 0 : scroller.scrollLeft);
      const right = edges[rightColumn + 1]! - (fixed ? 0 : scroller.scrollLeft);
      const clipLeft = Math.max(left, fixed ? edges[this.rowHeader ? 1 : 0]! : boundary);
      const clipRight = Math.min(right, fixed ? boundary : scroller.clientWidth);
      if (clipRight <= clipLeft) return;
      const top = first.getElement().getBoundingClientRect().top - viewport.top;
      const bottom = last.getElement().getBoundingClientRect().bottom - viewport.top;
      Object.assign(element.style, { display: "block", left: `${left}px`, top: `${top}px`, width: `${right - left}px`, height: `${bottom - top}px`,
        clipPath: `inset(0 ${right - clipRight}px 0 ${clipLeft - left}px)`,
        borderTopWidth: first.position - 1 === bounds.top ? "" : "0px",
        borderBottomWidth: last.position - 1 === bounds.bottom ? "" : "0px" });
    };
    for (const range of this.ranges) {
      let parts = fragments.get(range);
      if (!parts) {
        range.element.style.display = "contents";
        parts = [true, false].map((fixed) => {
          const element = this.table.element.ownerDocument.createElement("div");
          element.className = "tabulator-range csv-range-fragment";
          element.dataset.frozen = String(fixed);
          range.element.appendChild(element);
          return element;
        });
        fragments.set(range, parts);
      }
      draw(parts[0]!, range, true);
      draw(parts[1]!, range, false);
    }
    const start = this.activeRange && this.activeRange.start;
    this.activeRangeCellElement.style.display = "none";
    if (start) draw(this.activeRangeCellElement, { top: start.row, bottom: start.row, left: start.col, right: start.col }, start.col < frozenCount);
  };

  module.layoutChange = function () {
    if (destroyed || frame) return;
    // Use the current owner window so detached workbench panes keep painting.
    const window = this.table.element.ownerDocument.defaultView;
    if (window) frame = { window, id: window.requestAnimationFrame(() => { frame = null; this.layoutRanges(); }) };
  };
  module.autoScroll = function (range) {
    if (!supported) { originalScroll.call(this, range); return; }
    const scroller = this.table.rowManager.element;
    const columns = this.getTableColumns();
    scrollColumn(scroller, range.end.col, columnGeometry(columns, (index) => !!columns[index]!.definition.frozen));
    const row = this.getRowByRangePos(range.end.row)?.getElement();
    if (row) {
      if (row.offsetTop < scroller.scrollTop) scroller.scrollTop = row.offsetTop;
      else if (row.offsetTop + row.offsetHeight > scroller.scrollTop + scroller.clientHeight) scroller.scrollTop = row.offsetTop + row.offsetHeight - scroller.clientHeight;
    }
  };
  table.on("tableDestroyed", () => {
    destroyed = true;
    table.element.removeEventListener("mousedown", beginGesture, true);
    releaseGesture();
    if (frame) frame.window.cancelAnimationFrame(frame.id);
    frame = null;
  });
}
