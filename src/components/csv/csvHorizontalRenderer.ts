// Tabulator exposes custom renderers but does not ship types for its base classes.
// @ts-expect-error The runtime module is exported by tabulator-tables.
import VirtualDomHorizontal from "tabulator-tables/src/js/core/rendering/renderers/VirtualDomHorizontal.js";

const HorizontalRenderer = VirtualDomHorizontal as new (...args: unknown[]) => {
  initialize(): void;
  subscribe(event: string, callback: (force?: boolean) => boolean | void): void;
  options(key: string): unknown;
  rerenderColumns(update: boolean, blockRedraw: boolean): void;
  windowBuffer: number;
};

export class CsvHorizontalRenderer extends HorizontalRenderer {
  initialize(): void {
    super.initialize();
    // redraw() renders rows immediately afterwards. Update the column window
    // without first rebuilding the same rows in rerenderColumns().
    this.subscribe("table-redraw", () => this.rerenderColumns(true, true));
    // With explicit CSV widths, fitData has no column sizing work on a soft
    // redraw. Its variable-height pass would measure *every* active row before
    // the vertical renderer measures the visible rows again. Keep forced layout
    // for column/data changes; viewport resizing needs only the latter pass.
    this.subscribe("table-redrawing", (force) => !force && this.options("layout") === "fitData");
  }

  calcWindowBuffer(): void {
    // The default keeps two viewport widths on each side, multiplying the DOM
    // involved in row measurement. A small pixel margin still covers scrolling.
    this.windowBuffer = 200;
  }
}
