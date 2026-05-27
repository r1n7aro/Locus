pub(super) fn app_vue(_name: &str) -> String {
    r##"<template>
  <main class="view-shell inspector-view">
    <header class="view-toolbar">
      <button type="button">Apply</button>
    </header>

    <section class="inspector-grid">
      <label>
        <span>Target</span>
        <input value="Assets/Materials/Example.mat" />
      </label>
      <label>
        <span>Base Color</span>
        <input value="#d9dde5" />
      </label>
      <label>
        <span>Metallic</span>
        <input value="0.00" />
      </label>
      <label>
        <span>Smoothness</span>
        <input value="0.50" />
      </label>
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

button {
  min-height: 30px;
  padding: 0 12px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-color);
  font: inherit;
}

.inspector-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 10px;
  max-width: 620px;
  padding-top: 16px;
}

label {
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  font-size: 13px;
}

label span {
  color: var(--text-secondary);
}

input {
  min-height: 30px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
}
"#
    .to_string()
}

pub(super) fn view_api_cs() -> String {
    r#"using System;
using UnityEditor;
using UnityEngine;

public static class InspectorViewApi
{
    public static object Read(object args)
    {
        return new
        {
            ok = true,
            message = "View Script runtime is ready for this package."
        };
    }
}
"#
    .to_string()
}
