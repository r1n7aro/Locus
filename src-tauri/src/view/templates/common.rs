pub(super) fn main_ts() -> String {
    r##"import { createApp } from "vue";
import App from "./App.vue";
import "./style.css";

createApp(App).mount("#app");
"##
    .to_string()
}

/// Shared base stylesheet prepended to every template's `style.css`.
/// Keeps shell, toolbar, and control sizing identical across templates so
/// generated Views look consistent; templates append their own rules after it.
pub(super) fn base_css() -> &'static str {
    r#":root {
  color-scheme: light dark;
  font-family: var(--font-ui);
}

body {
  margin: 0;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

html,
body,
#app {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
}

.view-shell {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-color);
}

.view-toolbar {
  min-height: 42px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 10px 0 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
}

.toolbar-title {
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.toolbar-title span {
  font-size: 13px;
  font-weight: 650;
}

.toolbar-title small {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 11px;
}

.toolbar-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

button {
  min-height: 28px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}

button:hover:not(:disabled) {
  background: var(--hover-bg);
  border-color: var(--border-strong);
}

button:disabled {
  cursor: default;
  opacity: 0.55;
}

input,
select {
  min-height: 26px;
  min-width: 0;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  box-sizing: border-box;
}

input:focus,
select:focus {
  outline: none;
  border-color: var(--accent-color);
}
"#
}

pub(super) fn style_css(template_css: &str) -> String {
    format!("{}\n{}", base_css(), template_css)
}
