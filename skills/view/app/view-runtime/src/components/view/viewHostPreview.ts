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
}`;
}

// The preview is a static, script-free snapshot of the package template. Live
// behavior always comes from the compiled SFC in the View host, so the preview
// must not inject per-template runtime shims that the real host never loads.
export function buildViewPreviewSrcdoc(detail: ViewPackageDetail | null): string {
  if (!detail) return "";
  const appSource = viewFileContent(detail, "src/App.vue");
  const styleSource = viewFileContent(detail, detail.manifest.style);
  const template = stripScripts(extractVueTemplate(appSource));
  const safeCss = sanitizeCssForPreview(styleSource);
  const body = template || `<main class="view-preview-empty">${escapeHtml(detail.manifest.name)}</main>`;
  const runtimeBaseCss = locusViewRuntimeBaseCss();
  const runtimeCompatibilityCss = locusViewRuntimeCompatibilityCss();

  return `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:;">
  <style>${runtimeBaseCss}</style>
  <style>${safeCss}</style>
  <style>${runtimeCompatibilityCss}</style>
</head>
<body class="locus-view-runtime">${body}</body>
</html>`;
}
