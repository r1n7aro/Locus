pub(super) fn app_vue(_name: &str) -> String {
    r##"<template>
  <main class="view-shell link-board-view" data-locus-template="link-board">
    <header class="view-toolbar">
      <button type="button" data-link-save>Save Links</button>
    </header>

    <section class="link-board" data-link-board>
      <div class="link-column">
        <div class="link-column-title">Sources</div>
        <button type="button" class="link-item" data-link-source="albedo">Albedo Map</button>
        <button type="button" class="link-item" data-link-source="normal">Normal Map</button>
        <button type="button" class="link-item" data-link-source="mask">Mask Texture</button>
      </div>

      <svg class="link-lines" data-link-lines aria-hidden="true"></svg>

      <div class="link-column">
        <div class="link-column-title">Targets</div>
        <button type="button" class="link-item" data-link-target="_BaseMap">_BaseMap</button>
        <button type="button" class="link-item" data-link-target="_BumpMap">_BumpMap</button>
        <button type="button" class="link-item" data-link-target="_MaskMap">_MaskMap</button>
      </div>
    </section>

    <section class="link-data-panel">
      <div class="view-section-title">Link Data</div>
      <pre data-link-output></pre>
    </section>
  </main>
</template>
"##
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
  padding: 18px;
  box-sizing: border-box;
}

.view-toolbar {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--border-color);
}

.link-board {
  position: relative;
  min-height: 330px;
  display: grid;
  grid-template-columns: minmax(180px, 240px) minmax(120px, 1fr) minmax(180px, 240px);
  gap: 18px;
  margin-top: 14px;
  padding: 16px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
  overflow: hidden;
}

.link-column {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.link-column-title {
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.link-item {
  min-height: 36px;
  justify-content: flex-start;
  text-align: left;
}

.link-item.active,
.link-item.linked {
  border-color: var(--accent-color);
  background: var(--accent-soft);
}

.link-lines {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
}

.link-data-panel {
  margin-top: 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--panel-bg);
  overflow: hidden;
}

.view-section-title {
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

pre {
  margin: 0;
  min-height: 88px;
  padding: 10px;
  overflow: auto;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}
"#
    .to_string()
}
