import { parse as parseVueSfc } from "@vue/compiler-sfc";
import type { ViewPackageDetail } from "../../services/view";
import { sanitizeCssForPreview, viewFileContent } from "./viewPackageFiles";

export { sanitizeCssForPreview, viewFileContent, viewPackageRelPath } from "./viewPackageFiles";

const LOCUS_THEME_VARIABLES = [
  "--bg-color",
  "--sidebar-bg",
  "--panel-bg",
  "--surface-elevated",
  "--text-color",
  "--text-secondary",
  "--border-color",
  "--border-strong",
  "--hover-bg",
  "--active-bg",
  "--input-bg",
  "--accent-color",
  "--accent-soft",
  "--status-danger-fg",
  "--status-danger-bg",
  "--status-danger-border",
  "--font-ui",
  "--font-mono-identifier",
] as const;

const DEFAULT_LOCUS_THEME: Record<(typeof LOCUS_THEME_VARIABLES)[number], string> = {
  "--bg-color": "#1d1d21",
  "--sidebar-bg": "#17181c",
  "--panel-bg": "#111216",
  "--surface-elevated": "#1a1b20",
  "--text-color": "#f3f3f5",
  "--text-secondary": "#a1a4ad",
  "--border-color": "#26282e",
  "--border-strong": "#31333b",
  "--hover-bg": "#1d1f25",
  "--active-bg": "#23252c",
  "--input-bg": "#181a20",
  "--accent-color": "#6f77f6",
  "--accent-soft": "rgba(111, 119, 246, 0.12)",
  "--status-danger-fg": "#ff8a8a",
  "--status-danger-bg": "rgba(255, 138, 138, 0.14)",
  "--status-danger-border": "rgba(255, 138, 138, 0.30)",
  "--font-ui": "Inter, ui-sans-serif, system-ui, sans-serif",
  "--font-mono-identifier": "ui-monospace, SFMono-Regular, Cascadia Mono, Consolas, monospace",
};

export function extractVueTemplate(source: string): string {
  const parsed = parseVueSfc(source, { filename: "src/App.vue", sourceMap: false });
  if (parsed.errors.length) return "";
  return parsed.descriptor.template?.content.trim() || "";
}

function stripScripts(html: string): string {
  return html
    .replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, "")
    .replace(/\son[a-z]+\s*=\s*"[^"]*"/gi, "")
    .replace(/\son[a-z]+\s*=\s*'[^']*'/gi, "")
    .replace(/\son[a-z]+\s*=\s*[^\s>]+/gi, "");
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function sanitizeCssVariableValue(value: string): string {
  return value.replace(/[;\n\r{}]/g, "").trim();
}

function hostThemeDeclarations(): string {
  if (typeof window === "undefined" || typeof document === "undefined") {
    return "";
  }

  const styles = window.getComputedStyle(document.documentElement);
  return LOCUS_THEME_VARIABLES
    .map((name) => {
      const value = sanitizeCssVariableValue(styles.getPropertyValue(name));
      return value ? `  ${name}: ${value};` : "";
    })
    .filter(Boolean)
    .join("\n");
}

function defaultThemeDeclarations(): string {
  return LOCUS_THEME_VARIABLES
    .map((name) => `  ${name}: ${DEFAULT_LOCUS_THEME[name]};`)
    .join("\n");
}

function locusViewRuntimeBaseCss(): string {
  const hostTheme = hostThemeDeclarations();
  return `:root {
${defaultThemeDeclarations()}${hostTheme ? `\n${hostTheme}` : ""}
  color-scheme: light dark;
  font-family: var(--font-ui);
}

* {
  box-sizing: border-box;
}

html,
body {
  margin: 0;
  min-height: 100%;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
  font-size: 14px;
  line-height: 1.45;
  -webkit-font-smoothing: antialiased;
}

body.locus-view-runtime {
  overflow: auto;
}

button,
.locus-button {
  min-height: 30px;
  padding: 0 12px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-color);
  font: inherit;
  cursor: pointer;
}

button:hover,
.locus-button:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
}

button:focus-visible,
.locus-button:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent-color) 42%, transparent);
  outline-offset: 2px;
}

button:disabled,
.locus-button:disabled {
  cursor: default;
  opacity: 0.55;
}

input,
select,
textarea {
  min-height: 30px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
}

textarea {
  min-height: 80px;
  padding: 8px 9px;
}

input:focus,
select:focus,
textarea:focus {
  outline: none;
  border-color: var(--accent-color);
}

.locus-panel {
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
}`;
}

function locusViewRuntimeCompatibilityCss(): string {
  return `:root {
  font-family: var(--font-ui);
}

html,
body {
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

body.locus-view-runtime button,
body.locus-view-runtime .locus-button {
  border-color: var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-color);
}

body.locus-view-runtime input,
body.locus-view-runtime select,
body.locus-view-runtime textarea {
  border-color: var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
}

.view-shell,
.locus-view-shell {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 18px;
  background: var(--bg-color);
  color: var(--text-color);
}

.view-header,
.locus-view-header {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.view-toolbar,
.locus-view-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--border-color);
}

.view-kicker,
.locus-view-kicker {
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

h1 {
  margin: 0;
  color: var(--text-color);
  font-size: 18px;
  line-height: 1.25;
}

.view-panel {
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
  overflow: hidden;
}

.view-row {
  display: grid;
  grid-template-columns: 140px minmax(0, 1fr);
  gap: 12px;
  padding: 10px 12px;
  font-size: 13px;
}

.view-row + .view-row {
  border-top: 1px solid var(--border-color);
}

.view-row label,
.locus-field-label {
  color: var(--text-secondary);
}

.inspector-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 10px;
  max-width: 620px;
  padding-top: 16px;
}

.inspector-grid label {
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  color: var(--text-color);
  font-size: 13px;
}

.inspector-grid label span {
  color: var(--text-secondary);
}

.runtime-edge,
.runtime-link {
  stroke: var(--accent-color);
  stroke-width: 2;
  fill: none;
  vector-effect: non-scaling-stroke;
}

.runtime-edge {
  stroke-linecap: round;
}

.runtime-link {
  stroke-linecap: round;
  opacity: 0.9;
}`;
}

function escapeScript(value: string): string {
  return value.replace(/<\/script/gi, "<\\/script");
}

function locusViewRuntimeScript(): string {
  return `(function () {
  var SVG_NS = "http://www.w3.org/2000/svg";

  function clamp(value, min, max) {
    return Math.min(Math.max(value, min), max);
  }

  function centerPoint(element, container) {
    var rect = element.getBoundingClientRect();
    var containerRect = container.getBoundingClientRect();
    return {
      x: rect.left - containerRect.left + rect.width / 2,
      y: rect.top - containerRect.top + rect.height / 2
    };
  }

  function writeJson(target, value) {
    if (target) target.textContent = JSON.stringify(value, null, 2);
  }

  function initNodeGraph(root) {
    var canvas = root.querySelector("[data-graph-canvas]");
    var edgeLayer = root.querySelector("[data-graph-edges]");
    var output = root.querySelector("[data-graph-output]");
    if (!canvas || !edgeLayer) return;

    var nodes = Array.prototype.slice.call(canvas.querySelectorAll("[data-node-id]"));
    var edges = Array.prototype.slice.call(canvas.querySelectorAll("[data-graph-edge]"))
      .map(function (edge) {
        return {
          from: edge.getAttribute("data-edge-from") || "",
          to: edge.getAttribute("data-edge-to") || ""
        };
      })
      .filter(function (edge) { return edge.from && edge.to; });

    if (!edges.length && nodes.length > 1) {
      for (var i = 0; i < nodes.length - 1; i += 1) {
        edges.push({
          from: nodes[i].getAttribute("data-node-id") || "",
          to: nodes[i + 1].getAttribute("data-node-id") || ""
        });
      }
    }

    function nodeById(id) {
      return canvas.querySelector("[data-node-id=\\"" + id + "\\"]");
    }

    function graphData() {
      return {
        nodes: nodes.map(function (node) {
          return {
            id: node.getAttribute("data-node-id") || "",
            x: Math.round(parseFloat(node.style.left || "0") || node.offsetLeft || 0),
            y: Math.round(parseFloat(node.style.top || "0") || node.offsetTop || 0)
          };
        }),
        edges: edges.slice()
      };
    }

    function renderEdges() {
      edgeLayer.textContent = "";
      edges.forEach(function (edge) {
        var from = nodeById(edge.from);
        var to = nodeById(edge.to);
        if (!from || !to) return;
        var start = centerPoint(from, canvas);
        var end = centerPoint(to, canvas);
        var line = document.createElementNS(SVG_NS, "line");
        line.setAttribute("class", "runtime-edge");
        line.setAttribute("x1", String(start.x));
        line.setAttribute("y1", String(start.y));
        line.setAttribute("x2", String(end.x));
        line.setAttribute("y2", String(end.y));
        edgeLayer.appendChild(line);
      });
      writeJson(output, graphData());
    }

    nodes.forEach(function (node) {
      node.style.touchAction = "none";
      node.addEventListener("pointerdown", function (event) {
        if (event.button && event.button !== 0) return;
        var canvasRect = canvas.getBoundingClientRect();
        var nodeRect = node.getBoundingClientRect();
        var offsetX = event.clientX - nodeRect.left;
        var offsetY = event.clientY - nodeRect.top;
        node.setPointerCapture(event.pointerId);

        function onMove(moveEvent) {
          var nextX = moveEvent.clientX - canvasRect.left - offsetX;
          var nextY = moveEvent.clientY - canvasRect.top - offsetY;
          node.style.left = clamp(nextX, 8, canvas.clientWidth - node.offsetWidth - 8) + "px";
          node.style.top = clamp(nextY, 8, canvas.clientHeight - node.offsetHeight - 8) + "px";
          renderEdges();
        }

        function onUp(upEvent) {
          try { node.releasePointerCapture(upEvent.pointerId); } catch (_) {}
          node.removeEventListener("pointermove", onMove);
          node.removeEventListener("pointerup", onUp);
          node.removeEventListener("pointercancel", onUp);
          renderEdges();
        }

        node.addEventListener("pointermove", onMove);
        node.addEventListener("pointerup", onUp);
        node.addEventListener("pointercancel", onUp);
      });
    });

    var save = root.querySelector("[data-graph-save]");
    if (save) save.addEventListener("click", renderEdges);
    window.addEventListener("resize", renderEdges);
    renderEdges();
  }

  function initLinkBoard(root) {
    var board = root.querySelector("[data-link-board]");
    var lineLayer = root.querySelector("[data-link-lines]");
    var output = root.querySelector("[data-link-output]");
    if (!board || !lineLayer) return;

    var sources = Array.prototype.slice.call(board.querySelectorAll("[data-link-source]"));
    var targets = Array.prototype.slice.call(board.querySelectorAll("[data-link-target]"));
    var selectedSource = null;
    var connections = [];

    function sourceById(id) {
      return board.querySelector("[data-link-source=\\"" + id + "\\"]");
    }

    function targetById(id) {
      return board.querySelector("[data-link-target=\\"" + id + "\\"]");
    }

    function renderLinks() {
      lineLayer.textContent = "";
      sources.forEach(function (source) {
        var id = source.getAttribute("data-link-source") || "";
        source.classList.toggle("active", selectedSource === id);
        source.classList.toggle("linked", connections.some(function (item) { return item.source === id; }));
      });
      targets.forEach(function (target) {
        var id = target.getAttribute("data-link-target") || "";
        target.classList.toggle("linked", connections.some(function (item) { return item.target === id; }));
      });

      connections.forEach(function (connection) {
        var source = sourceById(connection.source);
        var target = targetById(connection.target);
        if (!source || !target) return;
        var start = centerPoint(source, board);
        var end = centerPoint(target, board);
        var dx = Math.max(48, Math.abs(end.x - start.x) / 2);
        var path = document.createElementNS(SVG_NS, "path");
        path.setAttribute("class", "runtime-link");
        path.setAttribute("d", "M " + start.x + " " + start.y + " C " + (start.x + dx) + " " + start.y + ", " + (end.x - dx) + " " + end.y + ", " + end.x + " " + end.y);
        lineLayer.appendChild(path);
      });

      writeJson(output, { connections: connections.slice() });
    }

    sources.forEach(function (source) {
      source.addEventListener("click", function () {
        selectedSource = source.getAttribute("data-link-source") || "";
        renderLinks();
      });
    });

    targets.forEach(function (target) {
      target.addEventListener("click", function () {
        if (!selectedSource) return;
        var targetId = target.getAttribute("data-link-target") || "";
        connections = connections.filter(function (item) {
          return item.source !== selectedSource && item.target !== targetId;
        });
        connections.push({ source: selectedSource, target: targetId });
        selectedSource = null;
        renderLinks();
      });
    });

    var save = root.querySelector("[data-link-save]");
    if (save) save.addEventListener("click", renderLinks);
    window.addEventListener("resize", renderLinks);
    renderLinks();
  }

  function init() {
    Array.prototype.slice.call(document.querySelectorAll('[data-locus-template="node-graph"]'))
      .forEach(initNodeGraph);
    Array.prototype.slice.call(document.querySelectorAll('[data-locus-template="link-board"]'))
      .forEach(initLinkBoard);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();`;
}

export function buildViewPreviewSrcdoc(detail: ViewPackageDetail | null): string {
  if (!detail) return "";
  const appSource = viewFileContent(detail, "src/App.vue");
  const styleSource = viewFileContent(detail, detail.manifest.style);
  const template = stripScripts(extractVueTemplate(appSource));
  const safeCss = sanitizeCssForPreview(styleSource);
  const body = template || `<main class="view-preview-empty">${escapeHtml(detail.manifest.name)}</main>`;
  const runtimeBaseCss = locusViewRuntimeBaseCss();
  const runtimeCompatibilityCss = locusViewRuntimeCompatibilityCss();
  const runtimeScript = escapeScript(locusViewRuntimeScript());

  return `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:;">
  <style>${runtimeBaseCss}</style>
  <style>${safeCss}</style>
  <style>${runtimeCompatibilityCss}</style>
</head>
<body class="locus-view-runtime">${body}<script data-locus-view-runtime>${runtimeScript}</script></body>
</html>`;
}
