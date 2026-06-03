export interface CanvasPoint {
  x: number;
  y: number;
}

export interface CanvasViewport {
  x: number;
  y: number;
  scale: number;
}

export interface CanvasItem {
  id: string;
  x?: number;
  y?: number;
  width?: number;
  height?: number;
}

export interface CanvasEditBehavior {
  readonly?: boolean;
  allowSelect?: boolean;
  allowMove?: boolean;
  allowPan?: boolean;
  allowBoxSelect?: boolean;
  allowCreate?: boolean;
  allowDelete?: boolean;
  allowCopy?: boolean;
  allowPaste?: boolean;
  allowContextMenu?: boolean;
}

export interface CanvasSelectionEvent<T extends CanvasItem = CanvasItem> {
  itemIds: string[];
  items: T[];
  anchorItemId?: string;
  source: "pointer" | "keyboard" | "box" | "program";
}

export interface CanvasClipboardEvent<T extends CanvasItem = CanvasItem> {
  itemIds: string[];
  items: T[];
  viewport: CanvasViewport;
}

export interface CanvasContextMenuEvent<T extends CanvasItem = CanvasItem> {
  itemId?: string;
  item?: T;
  itemIds: string[];
  items: T[];
  x: number;
  y: number;
  clientX: number;
  clientY: number;
}

export interface CanvasItemMoveEvent<T extends CanvasItem = CanvasItem> {
  item: T;
  itemIds: string[];
  items: T[];
  x: number;
  y: number;
  didDrag: boolean;
}

export interface CanvasViewExpose {
  fitContent: () => void;
  scheduleRender: () => void;
  getItemElement: (id: string) => HTMLElement | null;
  viewport: CanvasViewport;
}
