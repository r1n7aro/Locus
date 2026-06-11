pub(super) fn app_vue(_name: &str) -> String {
    r##"<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, shallowRef } from "vue";
import { onEditorUpdate, property } from "@locus/view-runtime";
import type { UnityBoundPropertyTree } from "@locus/view-runtime";

const statusText = ref("Reading");
const selectionName = ref("");
const boundTree = shallowRef<UnityBoundPropertyTree | null>(null);

let unsubscribe: (() => void) | null = null;
let selectionKey = "";
let refreshToken = 0;

// drawDefaultEditor() renders the default inspector for the bound object and
// writes commits back through the binding, so no commit wiring is needed here.
// Use tree.require(path).draw() or propertyDrawers for custom field rendering.
const SelectionEditor = () => boundTree.value?.drawDefaultEditor() ?? null;

async function refreshSelection() {
  const token = ++refreshToken;
  statusText.value = "Reading";
  try {
    const tree = await property.fromPath("selection", {
      maxDepth: 3,
      maxArrayItems: 32,
    });
    if (token !== refreshToken) return;
    boundTree.value = tree;
    statusText.value = "Ready";
  } catch (error) {
    if (token !== refreshToken) return;
    boundTree.value = null;
    statusText.value = error instanceof Error ? error.message : String(error);
  }
}

onMounted(() => {
  void refreshSelection();
  void onEditorUpdate((event) => {
    const key = `${event.selection.instanceId}:${event.selection.path}`;
    if (key === selectionKey) return;
    selectionKey = key;
    selectionName.value = event.selection.name;
    void refreshSelection();
  }).then((dispose) => {
    unsubscribe = dispose;
  });
});

onBeforeUnmount(() => {
  unsubscribe?.();
  unsubscribe = null;
});
</script>

<template>
  <main class="view-shell inspector-view" data-locus-template="inspector-form">
    <header class="view-toolbar">
      <div class="toolbar-title">
        <span>Selection Inspector</span>
        <small>{{ selectionName ? `${selectionName} · ${statusText}` : statusText }}</small>
      </div>
      <div class="toolbar-actions">
        <button type="button" @click="refreshSelection">Refresh</button>
      </div>
    </header>

    <section class="inspector-panel">
      <div v-if="boundTree" class="inspector-editor">
        <SelectionEditor />
      </div>
      <div v-else class="inspector-state">
        <span>{{ statusText }}</span>
        <small>Select an object in the Unity Editor to inspect it here.</small>
      </div>
    </section>
  </main>
</template>
"##
    .to_string()
}

pub(super) fn style_css() -> String {
    super::common::style_css(
        r#".inspector-panel {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: auto;
  padding: 14px;
}

.inspector-editor {
  max-width: 680px;
}

.inspector-state {
  padding: 12px;
  color: var(--text-secondary);
  font-size: 12px;
}

.inspector-state small {
  display: block;
  margin-top: 4px;
  font-size: 11px;
}
"#,
    )
}

pub(super) fn view_api_cs() -> String {
    r#"using System;
using UnityEditor;
using UnityEngine;

// Optional View Script entry. Rename the class together with the
// scripts[] entry in view.json when this package needs custom C# logic.
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
