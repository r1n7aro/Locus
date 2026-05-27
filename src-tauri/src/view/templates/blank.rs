pub(super) fn app_vue(_name: &str) -> String {
    r#"<template>
  <main class="view-shell">
    <section class="view-panel">
      <div class="view-row">
        <label>Context</label>
        <span>Waiting for Unity data</span>
      </div>
      <div class="view-row">
        <label>Status</label>
        <span>Ready</span>
      </div>
    </section>
  </main>
</template>
"#
    .to_string()
}

pub(super) fn style_css() -> String {
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
  min-height: 100%;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 20px;
  box-sizing: border-box;
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

label {
  color: var(--text-secondary);
}
"#
    .to_string()
}
