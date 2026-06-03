import { onBeforeUnmount, onMounted } from "vue";

let canvasStyleEl: HTMLStyleElement | null = null;
let canvasStyleUsers = 0;

export function locusCanvasCss(): string {
  return `.locus-canvas-view {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.locus-canvas-viewport {
  position: relative;
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  cursor: grab;
  background:
    linear-gradient(var(--border-color) 1px, transparent 1px),
    linear-gradient(90deg, var(--border-color) 1px, transparent 1px),
    var(--panel-bg);
  background-size: 28px 28px;
}

.locus-canvas-viewport.no-grid {
  background: var(--panel-bg);
}

.locus-canvas-viewport:active {
  cursor: grabbing;
}

.locus-canvas-viewport.box-select-enabled {
  cursor: crosshair;
}

.locus-canvas-viewport.box-select-enabled:active,
.locus-canvas-viewport.selecting {
  cursor: crosshair;
}

.locus-canvas-world {
  position: absolute;
  inset: 0 auto auto 0;
  transform-origin: 0 0;
}

.locus-canvas-overlay-layer {
  position: absolute;
  inset: 0;
  overflow: visible;
  pointer-events: none;
  z-index: 1;
}

.locus-canvas-item {
  position: absolute;
  z-index: 2;
  box-sizing: border-box;
}

.locus-canvas-item:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent-color) 42%, transparent);
  outline-offset: 2px;
}

.locus-canvas-selection-box {
  position: absolute;
  z-index: 4;
  box-sizing: border-box;
  pointer-events: none;
  border: 1px solid color-mix(in srgb, var(--accent-color) 72%, transparent);
  background: color-mix(in srgb, var(--accent-color) 12%, transparent);
}`;
}

export function useLocusCanvasStyles() {
  onMounted(() => {
    canvasStyleUsers += 1;
    if (!canvasStyleEl) {
      canvasStyleEl = document.createElement("style");
      canvasStyleEl.dataset.locusCanvasStyle = "true";
      canvasStyleEl.textContent = locusCanvasCss();
      document.head.appendChild(canvasStyleEl);
    }
  });

  onBeforeUnmount(() => {
    canvasStyleUsers = Math.max(0, canvasStyleUsers - 1);
    if (canvasStyleUsers === 0) {
      canvasStyleEl?.remove();
      canvasStyleEl = null;
    }
  });
}
