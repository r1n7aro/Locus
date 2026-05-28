import { onBeforeUnmount, onMounted } from "vue";

let graphStyleEl: HTMLStyleElement | null = null;
let graphStyleUsers = 0;

export function locusGraphCss(): string {
  return `.locus-graph-view {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-color);
  color: var(--text-color);
  --locus-graph-edge-color-0: color-mix(in srgb, var(--accent-color) 84%, var(--text-color) 16%);
  --locus-graph-edge-color-1: color-mix(in srgb, var(--status-warn-fg) 88%, var(--text-color) 12%);
  --locus-graph-edge-color-2: color-mix(in srgb, var(--status-good-fg) 88%, var(--text-color) 12%);
  --locus-graph-edge-color-3: color-mix(in srgb, var(--status-danger-fg) 82%, var(--text-color) 18%);
  --locus-graph-edge-color-4: color-mix(in srgb, var(--text-color) 76%, var(--accent-color) 24%);
  --locus-graph-edge-color-5: color-mix(in srgb, var(--status-danger-fg) 58%, var(--status-warn-fg) 42%);
}

.locus-graph-toolbar {
  flex-shrink: 0;
  min-height: 44px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 7px 10px 7px 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--bg-color) 14%);
}

.locus-graph-heading {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.locus-graph-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 650;
}

.locus-graph-status {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 11px;
}

.locus-graph-actions {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 6px;
}

.locus-graph-layout-mode {
  width: 124px;
}

.locus-graph-error {
  flex-shrink: 0;
  padding: 7px 12px;
  border-bottom: 1px solid var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 12px;
}

.locus-graph-view > .locus-canvas-view {
  flex: 1;
  min-width: 0;
  min-height: 0;
}

.locus-graph-viewport {
  position: relative;
  flex: 1;
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

.locus-graph-viewport:active {
  cursor: grabbing;
}

.locus-graph-world {
  position: absolute;
  inset: 0 auto auto 0;
  transform-origin: 0 0;
}

.locus-graph-edge-layer {
  position: absolute;
  inset: 0;
  overflow: visible;
  pointer-events: none;
  z-index: 1;
}

.locus-graph-edge {
  fill: none;
  stroke: var(--locus-graph-edge-color-0);
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
  pointer-events: stroke;
  vector-effect: non-scaling-stroke;
}

.locus-graph-edge-direction {
  fill: none;
  stroke: var(--locus-graph-edge-color-0);
  stroke-width: 1.35;
  stroke-linecap: round;
  stroke-linejoin: round;
  pointer-events: none;
  vector-effect: non-scaling-stroke;
  opacity: 0.58;
}

.locus-graph-edge.route-color-0,
.locus-graph-edge-direction.route-color-0 {
  stroke: var(--locus-graph-edge-color-0);
}

.locus-graph-edge.route-color-1,
.locus-graph-edge-direction.route-color-1 {
  stroke: var(--locus-graph-edge-color-1);
}

.locus-graph-edge.route-color-2,
.locus-graph-edge-direction.route-color-2 {
  stroke: var(--locus-graph-edge-color-2);
}

.locus-graph-edge.route-color-3,
.locus-graph-edge-direction.route-color-3 {
  stroke: var(--locus-graph-edge-color-3);
}

.locus-graph-edge.route-color-4,
.locus-graph-edge-direction.route-color-4 {
  stroke: var(--locus-graph-edge-color-4);
}

.locus-graph-edge.route-color-5,
.locus-graph-edge-direction.route-color-5 {
  stroke: var(--locus-graph-edge-color-5);
}

.locus-graph-edge.has-route-overlap {
  stroke-width: 2.25;
}

.locus-graph-edge:hover,
.locus-graph-edge.selected {
  stroke: var(--accent-color);
  stroke-width: 3;
}

.locus-graph-edge-direction.selected {
  stroke: var(--accent-color);
  stroke-width: 1.55;
  opacity: 0.66;
}

.locus-graph-node {
  position: absolute;
  z-index: 2;
  --locus-graph-port-edge-offset: 16.5px;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  background: var(--surface-elevated);
  color: var(--text-color);
  box-shadow: 0 1px 0 color-mix(in srgb, var(--border-color) 70%, transparent);
  cursor: grab;
  user-select: none;
  overflow: visible;
}

.locus-graph-node * {
  box-sizing: border-box;
}

.locus-graph-node:active {
  cursor: grabbing;
}

.locus-graph-node:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent-color) 42%, transparent);
  outline-offset: 2px;
}

.locus-graph-node.selected {
  border-color: var(--accent-color);
}

.locus-graph-node.node-style-state {
  border-radius: 10px;
  background:
    linear-gradient(
      180deg,
      color-mix(in srgb, var(--surface-elevated) 88%, var(--sidebar-bg) 12%),
      color-mix(in srgb, var(--panel-bg) 84%, var(--surface-elevated) 16%)
    );
  box-shadow:
    inset 0 1px 0 color-mix(in srgb, var(--text-color) 8%, transparent),
    0 1px 0 color-mix(in srgb, var(--border-color) 60%, transparent);
}

.locus-graph-state-node {
  position: relative;
  width: 100%;
  height: 100%;
  min-height: 56px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 10px 20px;
}

.locus-graph-state-node-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 3px;
  text-align: center;
}

.locus-graph-state-node-title {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-size: 13px;
  font-weight: 650;
  line-height: 16px;
}

.locus-graph-state-node-subtitle {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 13px;
}

.locus-graph-node.node-style-state .locus-graph-port {
  position: absolute;
}

.locus-graph-node.node-style-state .locus-graph-port-side-left,
.locus-graph-node.node-style-state .locus-graph-port-side-right {
  top: 50%;
  transform: translateY(-50%);
}

.locus-graph-node.node-style-state .locus-graph-port-side-left {
  left: -7px;
}

.locus-graph-node.node-style-state .locus-graph-port-side-right {
  right: -7px;
}

.locus-graph-node.node-style-state .locus-graph-port-side-top,
.locus-graph-node.node-style-state .locus-graph-port-side-bottom {
  left: 50%;
  transform: translateX(-50%);
}

.locus-graph-node.node-style-state .locus-graph-port-side-top {
  top: -7px;
}

.locus-graph-node.node-style-state .locus-graph-port-side-bottom {
  bottom: -7px;
}

.locus-graph-node-header {
  height: 42px;
  min-height: 42px;
  display: grid;
  grid-template-columns: 18px minmax(0, 1fr) 18px;
  align-items: center;
  gap: 8px;
  padding: 6px 9px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--surface-elevated) 82%, var(--sidebar-bg) 18%);
  border-radius: 7px 7px 0 0;
}

.locus-graph-node-header.node-port-input:not(.node-port-output) {
  grid-template-columns: 18px minmax(0, 1fr);
}

.locus-graph-node-header.node-port-output:not(.node-port-input) {
  grid-template-columns: minmax(0, 1fr) 18px;
}

.locus-graph-node-header:not(.node-port-input):not(.node-port-output) {
  grid-template-columns: minmax(0, 1fr);
}

.locus-graph-node-header > .locus-graph-port-input {
  justify-self: start;
  transform: translateX(calc(-1 * var(--locus-graph-port-edge-offset)));
}

.locus-graph-node-header > .locus-graph-port-output {
  justify-self: end;
  transform: translateX(var(--locus-graph-port-edge-offset));
}

.locus-graph-node-title-block {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.locus-graph-node-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 650;
  line-height: 15px;
}

.locus-graph-node-subtitle {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 13px;
}

.locus-graph-node-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 8px 9px 10px;
}

.locus-graph-port-groups {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 10px;
}

.locus-graph-port-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.locus-graph-port-list.empty {
  pointer-events: none;
}

.locus-graph-port-list-output {
  align-items: flex-end;
}

.locus-graph-port-row {
  position: relative;
  width: 100%;
  height: 13px;
  min-height: 13px;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 4px;
}

.locus-graph-port-list-input .locus-graph-port-row {
  justify-content: flex-start;
}

.locus-graph-port-list-output .locus-graph-port-row {
  justify-content: flex-end;
}

.locus-graph-port-row .locus-graph-port {
  position: absolute;
  top: 0;
}

.locus-graph-port-row .locus-graph-port-input {
  left: calc(-1 * var(--locus-graph-port-edge-offset));
}

.locus-graph-port-row .locus-graph-port-output {
  right: calc(-1 * var(--locus-graph-port-edge-offset));
}

.locus-graph-port-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 13px;
}

.locus-graph-port-list-input .locus-graph-port-label {
  text-align: left;
}

.locus-graph-port-list-output .locus-graph-port-label {
  text-align: right;
}

.locus-graph-port {
  position: relative;
  z-index: 4;
  --locus-graph-port-fill: var(--locus-graph-edge-color-0);
  width: 13px;
  height: 13px;
  min-width: 13px;
  min-height: 13px;
  flex: 0 0 13px;
  padding: 0;
  border: 1px solid var(--border-strong);
  border-radius: 50%;
  background: var(--panel-bg);
  cursor: crosshair;
}

.locus-graph-port.route-color-0 {
  --locus-graph-port-fill: var(--locus-graph-edge-color-0);
}

.locus-graph-port.route-color-1 {
  --locus-graph-port-fill: var(--locus-graph-edge-color-1);
}

.locus-graph-port.route-color-2 {
  --locus-graph-port-fill: var(--locus-graph-edge-color-2);
}

.locus-graph-port.route-color-3 {
  --locus-graph-port-fill: var(--locus-graph-edge-color-3);
}

.locus-graph-port.route-color-4 {
  --locus-graph-port-fill: var(--locus-graph-edge-color-4);
}

.locus-graph-port.route-color-5 {
  --locus-graph-port-fill: var(--locus-graph-edge-color-5);
}

.locus-graph-port.connected {
  border-color: var(--locus-graph-port-fill);
  background: var(--locus-graph-port-fill);
  box-shadow: inset 0 0 0 2px var(--surface-elevated);
}

.locus-graph-port:hover,
.locus-graph-port.active {
  border-color: var(--accent-color);
  background: var(--accent-soft);
}

.locus-graph-parameters {
  display: flex;
  flex-direction: column;
  gap: 7px;
  padding-top: 8px;
  border-top: 1px solid var(--border-color);
}

.locus-graph-parameters.align-output {
  align-items: flex-end;
}

.locus-graph-parameters.align-input {
  align-items: flex-start;
}

.locus-graph-parameter {
  display: grid;
  grid-template-columns: minmax(74px, 0.42fr) minmax(0, 1fr);
  align-items: start;
  gap: 8px;
}

.locus-graph-parameter-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 16px;
  padding-top: 5px;
}

.locus-graph-parameters.align-output .locus-graph-parameter {
  width: 100%;
  grid-template-columns: minmax(74px, 0.42fr) minmax(0, 1fr);
}

.locus-graph-parameters.align-output .locus-graph-parameter-label {
  text-align: right;
}

.locus-graph-parameters.align-output .locus-graph-parameter input,
.locus-graph-parameters.align-output .locus-graph-parameter select,
.locus-graph-parameters.align-output .locus-graph-parameter textarea,
.locus-graph-parameters.align-output .locus-graph-parameter-value {
  justify-self: end;
}

.locus-graph-parameters.align-input .locus-graph-parameter-label {
  text-align: left;
}

.locus-graph-parameter input,
.locus-graph-parameter select {
  width: 100%;
  min-height: 26px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--surface-elevated) 22%);
  color: var(--text-color);
  font-family: inherit;
  font-size: 12px;
  line-height: 16px;
}

.locus-graph-parameter input:disabled,
.locus-graph-parameter select:disabled,
.locus-graph-parameter textarea:disabled {
  opacity: 1;
  color: var(--text-color);
  -webkit-text-fill-color: var(--text-color);
}

.locus-graph-parameter input[type="checkbox"] {
  width: 14px;
  min-height: 14px;
  justify-self: start;
}

.locus-graph-parameter input[type="color"] {
  padding: 2px;
}

.locus-graph-parameter textarea {
  width: 100%;
  min-height: 54px;
  padding: 6px 7px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--surface-elevated) 22%);
  color: var(--text-color);
  font-family: inherit;
  font-size: 12px;
  line-height: 16px;
}

.locus-graph-parameter-value {
  width: 100%;
  min-width: 0;
  min-height: 26px;
  padding: 4px 7px;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, transparent);
  border-radius: 4px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--surface-elevated) 18%);
  color: color-mix(in srgb, var(--text-color) 92%, var(--accent-color) 8%);
  font-family: var(--font-mono-inline);
  font-size: 12px;
  font-weight: 600;
  line-height: 16px;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  word-break: break-word;
}

.locus-graph-formula {
  display: block;
  font: inherit;
  color: inherit;
  white-space: inherit;
  overflow-wrap: anywhere;
  word-break: break-word;
}

.locus-graph-formula-token.token-identifier {
  color: var(--text-color);
}

.locus-graph-formula-token.token-number {
  color: color-mix(in srgb, var(--status-warn-fg) 88%, var(--text-color) 12%);
}

.locus-graph-formula-token.token-operator {
  color: color-mix(in srgb, var(--accent-color) 86%, var(--text-color) 14%);
  font-weight: 700;
}

.locus-graph-formula-token.token-punctuation {
  color: var(--text-secondary);
}

.locus-graph-formula-token.token-string {
  color: color-mix(in srgb, var(--status-good-fg) 88%, var(--text-color) 12%);
}

.locus-graph-formula-token.token-text {
  color: color-mix(in srgb, var(--text-color) 88%, var(--text-secondary) 12%);
}`;
}

export function useLocusGraphStyles() {
  onMounted(() => {
    graphStyleUsers += 1;
    if (!graphStyleEl) {
      graphStyleEl = document.createElement("style");
      graphStyleEl.dataset.locusGraphStyle = "true";
      graphStyleEl.textContent = locusGraphCss();
      document.head.appendChild(graphStyleEl);
    }
  });

  onBeforeUnmount(() => {
    graphStyleUsers = Math.max(0, graphStyleUsers - 1);
    if (graphStyleUsers === 0) {
      graphStyleEl?.remove();
      graphStyleEl = null;
    }
  });
}
