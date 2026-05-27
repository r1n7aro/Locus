import {
  defineComponent,
  h,
  markRaw,
  onBeforeUnmount,
  onBeforeUpdate,
  onMounted,
  reactive,
  ref,
  type CSSProperties,
  type PropType,
  type VNodeRef,
} from "vue";
import { useLocusCanvasStyles } from "./canvasStyles";
import type {
  CanvasClipboardEvent,
  CanvasContextMenuEvent,
  CanvasEditBehavior,
  CanvasItem,
  CanvasItemMoveEvent,
  CanvasPoint,
  CanvasSelectionEvent,
  CanvasViewExpose,
  CanvasViewport,
} from "./canvasTypes";

const CANVAS_WORLD_SIZE = 4096;
const CANVAS_DEFAULT_ITEM_WIDTH = 240;
const CANVAS_DEFAULT_ITEM_HEIGHT = 112;
const CANVAS_DRAG_THRESHOLD = 3;

type CanvasItemClass = string | string[] | Record<string, boolean> | Array<string | Record<string, boolean>>;
type ResolvedCanvasEditBehavior = Required<CanvasEditBehavior>;

interface CanvasItemBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface CanvasSelectionBox {
  start: CanvasPoint;
  current: CanvasPoint;
  startWorld: CanvasPoint;
  currentWorld: CanvasPoint;
  baseItemIds: string[];
  additive: boolean;
}

function shouldIgnoreCanvasDrag(target: EventTarget | null, selector: string): boolean {
  return target instanceof Element && !!target.closest(selector);
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  return !!target.closest("input, select, textarea, [contenteditable='true']");
}

function itemNumber(value: number | undefined, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function viewportSnapshot(viewport: CanvasViewport): CanvasViewport {
  return {
    x: viewport.x,
    y: viewport.y,
    scale: viewport.scale,
  };
}

function normalizeIds(itemIds: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const itemId of itemIds) {
    if (!itemId || seen.has(itemId)) continue;
    seen.add(itemId);
    result.push(itemId);
  }
  return result;
}

function rectIntersects(a: CanvasItemBounds, b: CanvasItemBounds): boolean {
  return a.x <= b.x + b.width
    && a.x + a.width >= b.x
    && a.y <= b.y + b.height
    && a.y + a.height >= b.y;
}

function createCanvasViewComponent() {
  return defineComponent({
    name: "LocusCanvasView",
    props: {
      items: {
        type: Array as PropType<CanvasItem[]>,
        default: () => [],
      },
      selectedItemId: {
        type: String,
        default: "",
      },
      selectedItemIds: {
        type: Array as PropType<string[]>,
        default: () => [],
      },
      selectionActive: {
        type: Boolean,
        default: false,
      },
      readonly: {
        type: Boolean,
        default: false,
      },
      moveReadonly: {
        type: Boolean,
        default: true,
      },
      editBehavior: {
        type: Object as PropType<CanvasEditBehavior>,
        default: undefined,
      },
      showGrid: {
        type: Boolean,
        default: true,
      },
      worldSize: {
        type: Number,
        default: CANVAS_WORLD_SIZE,
      },
      minScale: {
        type: Number,
        default: 0.35,
      },
      maxScale: {
        type: Number,
        default: 1.8,
      },
      fitPadding: {
        type: Number,
        default: 72,
      },
      ignoreDragSelector: {
        type: String,
        default: "input, select, textarea, button",
      },
      itemClass: {
        type: Function as PropType<(item: CanvasItem, selected: boolean) => CanvasItemClass>,
        default: undefined,
      },
      itemStyle: {
        type: Function as PropType<(item: CanvasItem, selected: boolean) => CSSProperties>,
        default: undefined,
      },
    },
    emits: [
      "update:selectedItemId",
      "update:selectedItemIds",
      "selectItem",
      "selectItems",
      "itemDragStart",
      "itemMove",
      "itemMoveEnd",
      "copySelection",
      "pasteSelection",
      "deleteSelection",
      "contextMenu",
      "viewportChange",
      "render",
    ],
    setup(props, { emit, expose, slots }) {
      useLocusCanvasStyles();

      const viewport = reactive<CanvasViewport>({ x: 24, y: 24, scale: 1 });
      const viewportEl = ref<HTMLElement | null>(null);
      const itemElements = new Map<string, HTMLElement>();
      const draggingItemId = ref("");
      const selectionBox = ref<CanvasSelectionBox | null>(null);
      let renderScheduled = false;
      let renderFrame = 0;

      function effectiveBehavior(): ResolvedCanvasEditBehavior {
        const configuredReadonly = props.editBehavior?.readonly ?? props.readonly;
        const explicitReadonly = props.editBehavior?.readonly !== undefined;
        const readonlyLocksEdits = configuredReadonly && explicitReadonly;
        return {
          readonly: configuredReadonly,
          allowSelect: props.editBehavior?.allowSelect ?? true,
          allowMove: readonlyLocksEdits ? false : props.editBehavior?.allowMove ?? (!configuredReadonly || (!explicitReadonly && props.moveReadonly)),
          allowPan: props.editBehavior?.allowPan ?? true,
          allowBoxSelect: props.editBehavior?.allowBoxSelect ?? false,
          allowCreate: configuredReadonly ? false : props.editBehavior?.allowCreate ?? true,
          allowDelete: configuredReadonly ? false : props.editBehavior?.allowDelete ?? true,
          allowCopy: props.editBehavior?.allowCopy ?? true,
          allowPaste: configuredReadonly ? false : props.editBehavior?.allowPaste ?? true,
          allowContextMenu: props.editBehavior?.allowContextMenu ?? true,
        };
      }

      function emitViewportChange() {
        emit("viewportChange", viewportSnapshot(viewport));
      }

      function scheduleRender() {
        if (renderScheduled) return;
        renderScheduled = true;
        const flush = () => {
          renderScheduled = false;
          renderFrame = 0;
          emit("render");
        };
        if (typeof window !== "undefined" && typeof window.requestAnimationFrame === "function") {
          renderFrame = window.requestAnimationFrame(flush);
        } else {
          globalThis.setTimeout(flush, 0);
        }
      }

      function normalizedSelectedItemIds() {
        return normalizeIds(props.selectedItemIds.length ? props.selectedItemIds : props.selectedItemId ? [props.selectedItemId] : []);
      }

      function itemById(itemId: string) {
        return props.items.find((item) => item.id === itemId) ?? null;
      }

      function itemsByIds(itemIds: string[]) {
        return itemIds
          .map((itemId) => itemById(itemId))
          .filter((item): item is CanvasItem => !!item);
      }

      function emitSelection(itemIds: string[], source: CanvasSelectionEvent["source"], anchorItemId?: string) {
        const normalized = normalizeIds(itemIds);
        const payload: CanvasSelectionEvent = {
          itemIds: normalized,
          items: itemsByIds(normalized),
          anchorItemId,
          source,
        };
        emit("update:selectedItemIds", normalized);
        emit("update:selectedItemId", normalized[0] ?? "");
        emit("selectItems", payload);
        emit("selectItem", normalized[0] ?? "");
      }

      function selectItem(itemId: string, source: CanvasSelectionEvent["source"] = "program") {
        emitSelection(itemId ? [itemId] : [], source, itemId || undefined);
      }

      function pointerSelection(event: PointerEvent | MouseEvent, item: CanvasItem) {
        const current = normalizedSelectedItemIds();
        if (event.ctrlKey || event.metaKey) {
          return current.includes(item.id)
            ? current.filter((itemId) => itemId !== item.id)
            : [...current, item.id];
        }
        if (event.shiftKey) {
          return current.includes(item.id) ? current : [...current, item.id];
        }
        return current.includes(item.id) && current.length > 1 ? current : [item.id];
      }

      function itemBounds(item: CanvasItem): CanvasItemBounds {
        const element = itemElements.get(item.id);
        return {
          x: itemNumber(item.x, 0),
          y: itemNumber(item.y, 0),
          width: itemNumber(item.width, element?.offsetWidth || CANVAS_DEFAULT_ITEM_WIDTH),
          height: itemNumber(item.height, element?.offsetHeight || CANVAS_DEFAULT_ITEM_HEIGHT),
        };
      }

      function viewportPointFromClient(clientX: number, clientY: number): CanvasPoint {
        const container = viewportEl.value;
        if (!container) return { x: clientX, y: clientY };
        const rect = container.getBoundingClientRect();
        return {
          x: clientX - rect.left,
          y: clientY - rect.top,
        };
      }

      function worldPointFromClient(clientX: number, clientY: number): CanvasPoint {
        const point = viewportPointFromClient(clientX, clientY);
        return {
          x: Math.round((point.x - viewport.x) / viewport.scale),
          y: Math.round((point.y - viewport.y) / viewport.scale),
        };
      }

      function fitContent() {
        const container = viewportEl.value;
        if (!container || !props.items.length) return;

        const bounds = props.items.reduce(
          (result, item) => {
            const next = itemBounds(item);
            return {
              minX: Math.min(result.minX, next.x),
              minY: Math.min(result.minY, next.y),
              maxX: Math.max(result.maxX, next.x + next.width),
              maxY: Math.max(result.maxY, next.y + next.height),
            };
          },
          { minX: Infinity, minY: Infinity, maxX: -Infinity, maxY: -Infinity },
        );

        const width = Math.max(bounds.maxX - bounds.minX, 1);
        const height = Math.max(bounds.maxY - bounds.minY, 1);
        const padding = Math.max(0, props.fitPadding);
        const nextScale = Math.min(
          1.25,
          Math.max(props.minScale, Math.min((container.clientWidth - padding) / width, (container.clientHeight - padding) / height)),
        );
        viewport.scale = Number.isFinite(nextScale) ? nextScale : 1;
        viewport.x = Math.round((container.clientWidth - width * viewport.scale) / 2 - bounds.minX * viewport.scale);
        viewport.y = Math.round((container.clientHeight - height * viewport.scale) / 2 - bounds.minY * viewport.scale);
        emitViewportChange();
        scheduleRender();
      }

      function emitMove(
        name: "itemDragStart" | "itemMove" | "itemMoveEnd",
        item: CanvasItem,
        itemIds: string[],
        didDrag: boolean,
      ) {
        const payload: CanvasItemMoveEvent = {
          item,
          itemIds,
          items: itemsByIds(itemIds),
          x: itemNumber(item.x, 0),
          y: itemNumber(item.y, 0),
          didDrag,
        };
        emit(name, payload);
      }

      function trySetPointerCapture(element: HTMLElement, pointerId: number) {
        try {
          element.setPointerCapture(pointerId);
        } catch {
          // Synthetic View automation pointer events do not always register as active browser pointers.
        }
      }

      function onItemPointerDown(event: PointerEvent, item: CanvasItem) {
        if (event.button !== 0 || shouldIgnoreCanvasDrag(event.target, props.ignoreDragSelector)) return;
        const behavior = effectiveBehavior();
        if (!behavior.allowSelect && !behavior.allowMove) return;
        event.preventDefault();
        event.stopPropagation();

        const nextSelection = behavior.allowSelect ? pointerSelection(event, item) : normalizedSelectedItemIds();
        if (behavior.allowSelect) emitSelection(nextSelection, "pointer", item.id);
        if (!behavior.allowMove || !nextSelection.includes(item.id)) return;

        const target = event.currentTarget as HTMLElement;
        const startX = event.clientX;
        const startY = event.clientY;
        const dragItemIds = normalizeIds(nextSelection);
        const dragItems = itemsByIds(dragItemIds);
        const startPositions = new Map(dragItems.map((dragItem) => [
          dragItem.id,
          {
            x: itemNumber(dragItem.x, 0),
            y: itemNumber(dragItem.y, 0),
          },
        ]));
        let didDrag = false;
        trySetPointerCapture(target, event.pointerId);

        const onMove = (moveEvent: PointerEvent) => {
          const deltaX = (moveEvent.clientX - startX) / viewport.scale;
          const deltaY = (moveEvent.clientY - startY) / viewport.scale;
          if (!didDrag && Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY) < CANVAS_DRAG_THRESHOLD) return;
          if (!didDrag) {
            didDrag = true;
            draggingItemId.value = item.id;
            emitMove("itemDragStart", item, dragItemIds, true);
          }
          for (const dragItem of dragItems) {
            const start = startPositions.get(dragItem.id);
            if (!start) continue;
            dragItem.x = Math.round(start.x + deltaX);
            dragItem.y = Math.round(start.y + deltaY);
          }
          emitMove("itemMove", item, dragItemIds, true);
          scheduleRender();
        };

        const onUp = (upEvent: PointerEvent) => {
          try {
            target.releasePointerCapture(upEvent.pointerId);
          } catch {
            // Pointer capture may already be released by the host WebView.
          }
          target.removeEventListener("pointermove", onMove);
          target.removeEventListener("pointerup", onUp);
          target.removeEventListener("pointercancel", onUp);
          draggingItemId.value = "";
          emitMove("itemMoveEnd", item, dragItemIds, didDrag);
          if (didDrag) scheduleRender();
        };

        target.addEventListener("pointermove", onMove);
        target.addEventListener("pointerup", onUp);
        target.addEventListener("pointercancel", onUp);
      }

      function worldSelectionBounds(box: CanvasSelectionBox): CanvasItemBounds {
        const minX = Math.min(box.startWorld.x, box.currentWorld.x);
        const minY = Math.min(box.startWorld.y, box.currentWorld.y);
        return {
          x: minX,
          y: minY,
          width: Math.abs(box.currentWorld.x - box.startWorld.x),
          height: Math.abs(box.currentWorld.y - box.startWorld.y),
        };
      }

      function itemIdsInSelectionBox(box: CanvasSelectionBox) {
        const bounds = worldSelectionBounds(box);
        const selected = props.items
          .filter((item) => rectIntersects(itemBounds(item), bounds))
          .map((item) => item.id);
        return box.additive ? normalizeIds([...box.baseItemIds, ...selected]) : selected;
      }

      function startBoxSelection(event: PointerEvent) {
        const viewportNode = viewportEl.value;
        if (!viewportNode) return;
        const additive = event.ctrlKey || event.metaKey || event.shiftKey;
        const start = viewportPointFromClient(event.clientX, event.clientY);
        const startWorld = worldPointFromClient(event.clientX, event.clientY);
        selectionBox.value = {
          start,
          current: start,
          startWorld,
          currentWorld: startWorld,
          baseItemIds: additive ? normalizedSelectedItemIds() : [],
          additive,
        };
        if (!additive) emitSelection([], "pointer");
        trySetPointerCapture(viewportNode, event.pointerId);

        const onMove = (moveEvent: PointerEvent) => {
          const box = selectionBox.value;
          if (!box) return;
          box.current = viewportPointFromClient(moveEvent.clientX, moveEvent.clientY);
          box.currentWorld = worldPointFromClient(moveEvent.clientX, moveEvent.clientY);
          emitSelection(itemIdsInSelectionBox(box), "box");
          scheduleRender();
        };
        const onUp = (upEvent: PointerEvent) => {
          try {
            viewportNode.releasePointerCapture(upEvent.pointerId);
          } catch {
            // Pointer capture may already be released by the host WebView.
          }
          const box = selectionBox.value;
          if (box) emitSelection(itemIdsInSelectionBox(box), "box");
          selectionBox.value = null;
          viewportNode.removeEventListener("pointermove", onMove);
          viewportNode.removeEventListener("pointerup", onUp);
          viewportNode.removeEventListener("pointercancel", onUp);
          scheduleRender();
        };

        viewportNode.addEventListener("pointermove", onMove);
        viewportNode.addEventListener("pointerup", onUp);
        viewportNode.addEventListener("pointercancel", onUp);
      }

      function startViewportPan(event: PointerEvent) {
        const behavior = effectiveBehavior();
        const viewportNode = viewportEl.value;
        if (!viewportNode || !behavior.allowPan) return;

        const startX = event.clientX;
        const startY = event.clientY;
        const viewportStartX = viewport.x;
        const viewportStartY = viewport.y;
        trySetPointerCapture(viewportNode, event.pointerId);

        const onMove = (moveEvent: PointerEvent) => {
          viewport.x = Math.round(viewportStartX + moveEvent.clientX - startX);
          viewport.y = Math.round(viewportStartY + moveEvent.clientY - startY);
          emitViewportChange();
          scheduleRender();
        };
        const onUp = (upEvent: PointerEvent) => {
          try {
            viewportNode.releasePointerCapture(upEvent.pointerId);
          } catch {
            // Pointer capture may already be released by the host WebView.
          }
          viewportNode.removeEventListener("pointermove", onMove);
          viewportNode.removeEventListener("pointerup", onUp);
          viewportNode.removeEventListener("pointercancel", onUp);
        };

        viewportNode.addEventListener("pointermove", onMove);
        viewportNode.addEventListener("pointerup", onUp);
        viewportNode.addEventListener("pointercancel", onUp);
      }

      function onViewportPointerDown(event: PointerEvent) {
        if ((event.button !== 0 && event.button !== 1) || shouldIgnoreCanvasDrag(event.target, props.ignoreDragSelector)) return;
        const targetElement = event.target instanceof Element ? event.target : null;
        if (targetElement?.closest(".locus-canvas-item")) return;
        const behavior = effectiveBehavior();
        if (!behavior.allowSelect && !behavior.allowPan && !behavior.allowBoxSelect) return;

        event.preventDefault();
        if (behavior.allowBoxSelect && event.button === 0 && !event.altKey) {
          startBoxSelection(event);
          return;
        }
        if (behavior.allowSelect && event.button === 0 && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
          selectItem("", "pointer");
        }
        startViewportPan(event);
      }

      function onWheel(event: WheelEvent) {
        const container = viewportEl.value;
        if (!container) return;
        event.preventDefault();
        const rect = container.getBoundingClientRect();
        const worldX = (event.clientX - rect.left - viewport.x) / viewport.scale;
        const worldY = (event.clientY - rect.top - viewport.y) / viewport.scale;
        const factor = event.deltaY < 0 ? 1.08 : 0.92;
        const nextScale = Math.min(props.maxScale, Math.max(props.minScale, viewport.scale * factor));
        viewport.x = Math.round(event.clientX - rect.left - worldX * nextScale);
        viewport.y = Math.round(event.clientY - rect.top - worldY * nextScale);
        viewport.scale = nextScale;
        emitViewportChange();
        scheduleRender();
      }

      function clipboardPayload(itemIds = normalizedSelectedItemIds()): CanvasClipboardEvent {
        return {
          itemIds,
          items: itemsByIds(itemIds),
          viewport: viewportSnapshot(viewport),
        };
      }

      function onKeydown(event: KeyboardEvent) {
        if (isEditableTarget(event.target)) return;
        const behavior = effectiveBehavior();
        const itemIds = normalizedSelectedItemIds();
        const key = event.key.toLowerCase();
        const modifier = event.ctrlKey || event.metaKey;

        if (modifier && key === "c" && behavior.allowCopy && itemIds.length) {
          event.preventDefault();
          emit("copySelection", clipboardPayload(itemIds));
          return;
        }
        if (modifier && key === "v" && behavior.allowPaste) {
          event.preventDefault();
          emit("pasteSelection", clipboardPayload(itemIds));
          return;
        }
        if ((event.key === "Delete" || event.key === "Backspace") && behavior.allowDelete && (props.selectionActive || itemIds.length)) {
          event.preventDefault();
          emit("deleteSelection", clipboardPayload(itemIds));
        }
      }

      function emitContextMenu(event: MouseEvent, item?: CanvasItem) {
        const behavior = effectiveBehavior();
        if (!behavior.allowContextMenu) return;
        event.preventDefault();
        event.stopPropagation();

        let itemIds = normalizedSelectedItemIds();
        if (item && !itemIds.includes(item.id)) {
          itemIds = [item.id];
          if (behavior.allowSelect) emitSelection(itemIds, "pointer", item.id);
        }
        const point = worldPointFromClient(event.clientX, event.clientY);
        const payload: CanvasContextMenuEvent = {
          itemId: item?.id,
          item,
          itemIds,
          items: itemsByIds(itemIds),
          x: point.x,
          y: point.y,
          clientX: event.clientX,
          clientY: event.clientY,
        };
        emit("contextMenu", payload);
      }

      function itemWrapperClass(item: CanvasItem) {
        const selected = normalizedSelectedItemIds().includes(item.id);
        return [
          "locus-canvas-item",
          selected ? "selected" : "",
          selected && normalizedSelectedItemIds().length > 1 ? "multi-selected" : "",
          draggingItemId.value === item.id ? "dragging" : "",
          props.itemClass?.(item, selected),
        ];
      }

      function itemWrapperStyle(item: CanvasItem): CSSProperties {
        const selected = normalizedSelectedItemIds().includes(item.id);
        return {
          left: `${itemNumber(item.x, 0)}px`,
          top: `${itemNumber(item.y, 0)}px`,
          width: typeof item.width === "number" ? `${item.width}px` : undefined,
          height: typeof item.height === "number" ? `${item.height}px` : undefined,
          ...(props.itemStyle?.(item, selected) ?? {}),
        };
      }

      function renderItem(item: CanvasItem) {
        const selectedIds = normalizedSelectedItemIds();
        const selected = selectedIds.includes(item.id);
        const setItemRef: VNodeRef = (element) => {
          if (element instanceof HTMLElement) itemElements.set(item.id, element);
        };
        return h("div", {
          key: item.id,
          class: itemWrapperClass(item),
          style: itemWrapperStyle(item),
          tabindex: 0,
          role: "button",
          ref: setItemRef,
          onPointerdown: (event: PointerEvent) => onItemPointerDown(event, item),
          onContextmenu: (event: MouseEvent) => emitContextMenu(event, item),
          onKeydown: (event: KeyboardEvent) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              selectItem(item.id, "keyboard");
            }
          },
        }, slots.default?.({
          item,
          selected,
          selectedIds,
          dragging: draggingItemId.value === item.id,
          viewport,
        }));
      }

      function renderSelectionBox() {
        const box = selectionBox.value;
        if (!box) return null;
        const left = Math.min(box.start.x, box.current.x);
        const top = Math.min(box.start.y, box.current.y);
        const width = Math.abs(box.current.x - box.start.x);
        const height = Math.abs(box.current.y - box.start.y);
        return h("div", {
          class: "locus-canvas-selection-box",
          style: {
            left: `${left}px`,
            top: `${top}px`,
            width: `${width}px`,
            height: `${height}px`,
          },
        });
      }

      onBeforeUpdate(() => {
        itemElements.clear();
      });

      onMounted(() => {
        window.addEventListener("resize", scheduleRender);
      });

      onBeforeUnmount(() => {
        if (renderFrame) window.cancelAnimationFrame(renderFrame);
        window.removeEventListener("resize", scheduleRender);
      });

      expose({
        fitContent,
        scheduleRender,
        getItemElement: (id: string) => itemElements.get(id) ?? null,
        viewport,
      } satisfies CanvasViewExpose);

      return () => {
        const behavior = effectiveBehavior();
        return h("div", { class: "locus-canvas-view" }, [
          h("div", {
            class: [
              "locus-canvas-viewport",
              props.showGrid ? "" : "no-grid",
              behavior.allowBoxSelect ? "box-select-enabled" : "",
              selectionBox.value ? "selecting" : "",
            ],
            ref: viewportEl,
            tabindex: 0,
            onPointerdown: onViewportPointerDown,
            onContextmenu: (event: MouseEvent) => emitContextMenu(event),
            onKeydown,
            onWheel,
          }, [
            h("div", {
              class: "locus-canvas-world",
              style: {
                width: `${props.worldSize}px`,
                height: `${props.worldSize}px`,
                transform: `translate(${viewport.x}px, ${viewport.y}px) scale(${viewport.scale})`,
              },
            }, [
              slots.overlay?.({
                viewport,
                worldSize: props.worldSize,
              }),
              props.items.map(renderItem),
            ]),
            renderSelectionBox(),
          ]),
        ]);
      };
    },
  });
}

export const LocusCanvasView = markRaw(createCanvasViewComponent());
