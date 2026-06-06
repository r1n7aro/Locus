pub(super) fn app_vue(_name: &str) -> String {
    r##"<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { property } from "@locus/view-runtime";
import { UnitySerializedPropertyTree } from "@locus/components";

type BoundTree = Awaited<ReturnType<typeof property.fromPath>>;
type BoundCommit = Parameters<BoundTree["writeCommit"]>[0];

const statusText = ref("Ready");
const boundTree = ref<BoundTree | null>(null);

const treeSource = computed(() => {
  const tree = boundTree.value;
  if (!tree) return null;
  return {
    id: tree.bindingId,
    targetId: tree.bindingId,
    snapshots: tree.snapshots,
    commit: async (commit: BoundCommit) => {
      statusText.value = "Saving";
      await tree.writeCommit(commit, { refresh: true });
      statusText.value = "Ready";
    },
  };
});

async function refreshSelectionName() {
  statusText.value = "Reading";
  try {
    boundTree.value = await property.fromPath("selection/property/m_Name", {
      maxDepth: 2,
      maxArrayItems: 32,
    });
    statusText.value = "Ready";
  } catch (error) {
    statusText.value = error instanceof Error ? error.message : String(error);
  }
}

onMounted(() => {
  void refreshSelectionName();
});
</script>

<template>
  <main class="view-shell inspector-view">
    <header class="view-toolbar">
      <div class="toolbar-title">
        <span>Selection Inspector</span>
        <small>{{ statusText }}</small>
      </div>
      <button type="button" @click="refreshSelectionName">Refresh</button>
    </header>

    <section class="inspector-panel">
      <UnitySerializedPropertyTree
        v-if="treeSource"
        :source="treeSource"
      />
      <div v-else class="inspector-state">{{ statusText }}</div>
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
  justify-content: space-between;
  gap: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--border-color);
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
  color: var(--text-secondary);
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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

.inspector-panel {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  max-width: 620px;
  padding-top: 16px;
}

.inspector-state {
  padding: 12px;
  color: var(--text-secondary);
  font-size: 12px;
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
