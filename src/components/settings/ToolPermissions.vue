<script setup lang="ts">
import { t } from "../../i18n";
import BaseSegmented from "../ui/BaseSegmented.vue";

type ToolMode = "auto" | "ask";

interface ToolPermissionItem {
  name: string;
  label: string;
  desc: string;
  defaultMode: ToolMode;
}

const props = defineProps<{
  toolPermissionMode: ToolMode;
  toolList: ToolPermissionItem[];
  toolPermissions: Record<string, ToolMode>;
  permSaveMsg: string;
}>();

const emit = defineEmits<{
  setGlobalPermissionMode: [mode: ToolMode];
  setPermission: [name: string, mode: ToolMode];
}>();

const permissionOptions = [
  { value: "auto", label: "Auto" },
  { value: "ask", label: "Ask" },
] as const;

function getToolMode(name: string): ToolMode {
  return props.toolPermissions[name] ?? (props.toolList.find((tool) => tool.name === name)?.defaultMode ?? "ask");
}
</script>

<template>
  <div class="settings-section">
    <div class="perm-shell">
      <div class="perm-header">
        <div class="perm-heading">
          <div class="section-label">{{ t("settings.perms.title") }}</div>
          <p class="section-desc">{{ t("settings.perms.desc") }}</p>
        </div>

        <Transition name="fade">
          <div
            v-if="permSaveMsg"
            class="perm-toast"
            role="status"
            aria-live="polite"
          >
            {{ permSaveMsg }}
          </div>
        </Transition>
      </div>

      <div class="perm-mode-row">
        <div class="perm-mode-copy">
          <span class="perm-mode-label">{{ t("settings.perms.globalMode") }}</span>
          <span class="perm-mode-desc">{{ t("settings.perms.globalModeDesc") }}</span>
        </div>

        <div class="perm-mode-control">
          <BaseSegmented
            size="sm"
            :model-value="toolPermissionMode"
            :options="[...permissionOptions]"
            @update:model-value="emit('setGlobalPermissionMode', $event as ToolMode)"
          />
        </div>
      </div>

      <div class="perm-card">
        <div class="perm-table-head" aria-hidden="true">
          <span>{{ t("settings.perms.columnTool") }}</span>
          <span>{{ t("settings.perms.columnMode") }}</span>
        </div>

        <div class="perm-list">
          <div
            v-for="tool in toolList"
            :key="tool.name"
            class="perm-row"
          >
            <div class="perm-info">
              <span class="perm-name">{{ tool.label }}</span>
              <span class="perm-desc">{{ tool.desc }}</span>
            </div>

            <div class="perm-control">
              <BaseSegmented
                size="sm"
                :model-value="getToolMode(tool.name)"
                :options="[...permissionOptions]"
                @update:model-value="emit('setPermission', tool.name, $event as ToolMode)"
              />
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.perm-shell {
  position: relative;
}

.perm-header {
  position: relative;
  padding-right: 88px;
}

.perm-toast {
  position: absolute;
  top: 0;
  right: 0;
  display: inline-flex;
  align-items: center;
  min-height: 26px;
  padding: 0 10px;
  border-radius: 6px;
  border: 1px solid var(--status-good-border);
  background: color-mix(in srgb, var(--status-good-bg) 84%, var(--panel-bg) 16%);
  color: var(--status-good-fg);
  font-size: 11px;
  font-weight: 600;
  line-height: 1;
  pointer-events: none;
}

.perm-mode-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 16px;
  align-items: center;
  margin-bottom: 12px;
  padding: 12px 16px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
}

.perm-mode-copy {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.perm-mode-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.perm-mode-desc {
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.45;
}

.perm-mode-control {
  width: 116px;
  flex-shrink: 0;
}

.perm-mode-control :deep(.base-segmented) {
  display: flex;
  width: 100%;
}

.perm-mode-control :deep(.base-segmented-item) {
  flex: 1;
  justify-content: center;
}

.perm-card {
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
  overflow: hidden;
}

.perm-table-head {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 16px;
  align-items: center;
  padding: 10px 16px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--hover-bg) 22%);
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-secondary);
}

.perm-list {
  display: flex;
  flex-direction: column;
}

.perm-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 16px;
  align-items: center;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border-color);
  transition: background 0.15s ease;
}

.perm-row:last-child {
  border-bottom: none;
}

.perm-row:hover {
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--hover-bg) 18%);
}

.perm-info {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.perm-name {
  font-size: 13px;
  font-weight: 600;
  font-family: var(--font-mono-identifier);
  color: var(--text-color);
}

.perm-desc {
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.45;
}

.perm-control {
  width: 116px;
  flex-shrink: 0;
}

.perm-control :deep(.base-segmented) {
  display: flex;
  width: 100%;
}

.perm-control :deep(.base-segmented-item) {
  flex: 1;
  justify-content: center;
}

@media (max-width: 860px) {
  .perm-header {
    padding-right: 0;
  }

  .perm-toast {
    position: static;
    margin-bottom: 12px;
  }

  .perm-table-head {
    display: none;
  }

  .perm-mode-row {
    grid-template-columns: 1fr;
    gap: 10px;
  }

  .perm-mode-control {
    width: 100%;
  }

  .perm-row {
    grid-template-columns: 1fr;
    gap: 10px;
  }

  .perm-control {
    width: 100%;
  }
}
</style>
