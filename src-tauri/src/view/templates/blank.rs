pub(super) fn app_vue(_name: &str) -> String {
    r##"<script setup lang="ts">
import { ref } from "vue";

const statusText = ref("Ready");
</script>

<template>
  <main class="view-shell" data-locus-template="blank">
    <header class="view-toolbar">
      <div class="toolbar-title">
        <span>View</span>
        <small>{{ statusText }}</small>
      </div>
    </header>

    <section class="view-content">
      <section class="view-panel">
        <div class="view-row">
          <label>Context</label>
          <span>Waiting for data</span>
        </div>
        <div class="view-row">
          <label>Status</label>
          <span>{{ statusText }}</span>
        </div>
      </section>
    </section>
  </main>
</template>
"##
    .to_string()
}

pub(super) fn style_css() -> String {
    super::common::style_css(
        r#".view-content {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 14px;
  overflow: auto;
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
"#,
    )
}
