<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import {
  closeBlenderBackground,
  installBlenderAddon,
  reloadBlenderHosts,
  setBlenderSessionTarget,
  type BlenderInstance,
  type BlenderWorkspaceStatus,
} from "../../services/blender";
import { blenderErrorText } from "../../services/blenderErrors";
import type { WorkspaceRef } from "../../services/project";

const props = defineProps<{
  status: BlenderWorkspaceStatus | null;
  workspaceRef: WorkspaceRef | null;
  sessionId: string | null;
  /**
   * Target picked before the session exists. The host sends it with the
   * first message; `undefined` means the host cannot, so a draft offers no
   * target action.
   */
  draftTarget?: string | null;
  /** Lazy service not started yet: running Blenders are not connected. */
  dormant?: boolean;
  connecting?: boolean;
}>();

const emit = defineEmits<{ changed: []; connect: []; "update:draftTarget": [key: string | null] }>();

/** No session yet, but the host carries the pick into the first message. */
const draftMode = computed(() => !props.sessionId && props.draftTarget !== undefined);
const canTarget = computed(() => Boolean(props.sessionId) || draftMode.value);

const busyAction = ref("");
const actionError = ref("");
/** Instances a hot reload could not reach (not connected, old protocol). */
const restartPids = ref(new Set<number>());

const instances = computed(() => props.status?.instances ?? []);
const workspaceInstances = computed(() => instances.value.filter((item) => item.inWorkspace));
const otherInstances = computed(() => instances.value.filter((item) => !item.inWorkspace));
const backgroundPids = computed(() => new Set((props.status?.background ?? []).map((item) => item.pid)));
/** Background processes Locus started that have not registered yet. */
const startingBackground = computed(() => {
  const listed = new Set(instances.value.map((item) => item.pid));
  return (props.status?.background ?? []).filter((item) => !listed.has(item.pid));
});
const withoutHost = computed(() => props.status?.withoutHost ?? []);
const version = computed(() => props.status?.workspaceVersion ?? null);
const addon = computed(() => props.status?.addon ?? null);

const versionText = computed(() => {
  const value = version.value;
  if (!value) return "";
  return value.effective
    ? t(value.pinned ? "blender.status.versionPinned" : "blender.status.versionInferred", value.effective)
    : t("blender.status.versionUnknown");
});

/** Saved-version distribution of the workspace files, shown on hover. */
const versionTitle = computed(() => (version.value?.distribution.versions ?? [])
  .map(([name, count]) => t("blender.status.versionCount", name, count))
  .join("\n"));

const addonLine = computed(() => {
  const value = addon.value;
  const label = value?.installation?.displayVersion ?? "";
  if (value?.status === "missing") return { text: t("blender.status.addonMissing", label), action: "install" as const };
  if (value?.status === "outdated") return { text: t("blender.status.addonOutdated", label), action: "install" as const };
  // Installing reloads running hosts itself; a separate reload is only for
  // instances still on old code after an install elsewhere.
  const outdated = value?.outdatedInstances ?? [];
  if (outdated.length === 0) return null;
  if (outdated.every((pid) => restartPids.value.has(pid))) {
    return { text: t("blender.status.restartNeeded", outdated.length), action: null };
  }
  return { text: t("blender.status.outdatedHosts", outdated.length), action: "reload" as const };
});

function noteReload(reloaded: number[], restartNeeded: number[]) {
  const next = new Set(restartPids.value);
  for (const pid of reloaded) next.delete(pid);
  for (const pid of restartNeeded) next.add(pid);
  restartPids.value = next;
}

function fileLabel(instance: BlenderInstance): string {
  if (!instance.file) return t("blender.status.unsaved");
  const parts = instance.file.split(/[\\/]/);
  return `${parts[parts.length - 1] || instance.file}${instance.dirty ? "*" : ""}`;
}

function majorMinor(value: string): string {
  return value.split(/[^0-9]+/).filter(Boolean).slice(0, 2).join(".");
}

function instanceMeta(instance: BlenderInstance): string {
  const parts = [instance.version, `PID ${instance.pid}`];
  if (instance.mode === "background") parts.push(t("blender.status.background"));
  return parts.join(" · ");
}

function instanceTitle(instance: BlenderInstance): string {
  return [instance.file, `Blender ${instance.versionString || instance.version}`, instance.binary]
    .filter(Boolean)
    .join("\n");
}

/** Only states worth attention; a connected, idle instance has none. */
function instanceNote(instance: BlenderInstance): string {
  const notes: string[] = [];
  if (instance.connection === "host_error") notes.push(instance.error || t("blender.status.hostError"));
  else if (instance.connection !== "connected") notes.push(t(`blender.connection.${instance.connection}`));
  if ((instance.mainThreadStallMs ?? 0) >= 30_000) {
    notes.push(t("blender.status.stuck", Math.round((instance.mainThreadStallMs ?? 0) / 1000)));
  } else if (instance.busy) {
    notes.push(t("blender.status.busy"));
  }
  if (instance.sharedFile) notes.push(t("blender.status.sharedFile"));
  const wanted = version.value?.effective;
  if (wanted && majorMinor(wanted) !== majorMinor(instance.version)) {
    notes.push(t("blender.status.versionMismatch", wanted));
  }
  return notes.join(" · ");
}

function dotClass(instance: BlenderInstance): string {
  if (instance.connection === "host_error") return "is-danger";
  if (instance.connection !== "connected" || (instance.mainThreadStallMs ?? 0) >= 30_000) return "is-warning";
  if (instance.busy) return "is-busy";
  return "is-ok";
}

async function run(key: string, action: () => Promise<unknown>) {
  if (busyAction.value) return;
  busyAction.value = key;
  actionError.value = "";
  try {
    await action();
    emit("changed");
  } catch (error) {
    actionError.value = blenderErrorText(error);
  } finally {
    busyAction.value = "";
  }
}

function isTarget(instance: BlenderInstance): boolean {
  return draftMode.value ? instance.key === props.draftTarget : instance.isTarget;
}

function toggleTarget(instance: BlenderInstance) {
  if (draftMode.value) {
    actionError.value = "";
    emit("update:draftTarget", isTarget(instance) ? null : instance.key);
    return;
  }
  const workspaceRef = props.workspaceRef;
  const sessionId = props.sessionId;
  if (!workspaceRef || !sessionId) return;
  void run(`target:${instance.key}`, () => setBlenderSessionTarget(
    workspaceRef,
    sessionId,
    instance.isTarget ? null : instance.key,
  ));
}

function canClose(instance: BlenderInstance): boolean {
  return instance.ownedBackground || backgroundPids.value.has(instance.pid);
}

function closeBackground(pid: number) {
  void run(`close:${pid}`, () => closeBlenderBackground(pid));
}

function runAddonAction() {
  if (addonLine.value?.action === "reload") {
    void run("reload", async () => {
      const result = await reloadBlenderHosts();
      noteReload(result.reloaded, result.restartNeeded);
    });
    return;
  }
  const id = addon.value?.installation?.id ?? null;
  void run("install", async () => {
    const reports = await installBlenderAddon(id);
    for (const report of reports) noteReload(report.reloaded, report.restartNeeded);
    const failed = reports.find((report) => !report.ok);
    if (failed) {
      throw { code: "blender.install_failed", message: "", detail: failed.error ?? "", retryable: false, severity: "error" };
    }
  });
}
</script>

<template>
  <div class="blender-status-panel">
    <div class="blender-list">
      <div v-if="dormant" class="blender-empty blender-dormant">
        <span>{{ t("blender.status.dormantHint") }}</span>
        <button type="button" class="blender-link is-accent" :disabled="connecting" @click="emit('connect')">
          {{ connecting ? t("blender.status.connecting") : t("blender.status.connect") }}
        </button>
      </div>
      <div v-else-if="workspaceInstances.length === 0 && startingBackground.length === 0" class="blender-empty">
        {{ t("blender.status.noWorkspaceInstance") }}
      </div>
      <div
        v-for="instance in workspaceInstances"
        :key="instance.key"
        class="blender-row"
        :title="instanceTitle(instance)"
      >
        <span class="blender-dot" :class="dotClass(instance)" aria-hidden="true" />
        <div class="blender-row-main">
          <div class="blender-row-line">
            <span class="blender-file">{{ fileLabel(instance) }}</span>
            <span class="blender-meta">{{ instanceMeta(instance) }}</span>
            <span v-if="isTarget(instance)" class="blender-tag">{{ t("blender.status.target") }}</span>
          </div>
          <div v-if="instanceNote(instance) || (instance.otherSessionTarget && !isTarget(instance))" class="blender-sub">
            <span v-if="instance.otherSessionTarget && !isTarget(instance)" class="blender-subtle">{{ t("blender.status.otherSessionTarget") }}</span>
            <span v-if="instanceNote(instance)" class="blender-note">{{ instanceNote(instance) }}</span>
          </div>
        </div>
        <div class="blender-row-actions">
          <button
            v-if="canTarget"
            type="button"
            class="blender-link"
            :disabled="Boolean(busyAction)"
            @click="toggleTarget(instance)"
          >
            {{ isTarget(instance) ? t("blender.status.clearTarget") : t("blender.status.setTarget") }}
          </button>
          <button
            v-if="canClose(instance)"
            type="button"
            class="blender-link"
            :disabled="Boolean(busyAction)"
            @click="closeBackground(instance.pid)"
          >
            {{ t("blender.status.close") }}
          </button>
        </div>
      </div>
      <div v-for="item in startingBackground" :key="`bg-${item.pid}`" class="blender-row">
        <span class="blender-dot is-warning" aria-hidden="true" />
        <div class="blender-row-main">
          <div class="blender-row-line">
            <span class="blender-file">{{ item.file ? item.file.split(/[\\/]/).pop() : t("blender.status.noFile") }}</span>
            <span class="blender-meta">{{ item.version }} · PID {{ item.pid }} · {{ t("blender.status.background") }}</span>
          </div>
        </div>
        <div class="blender-row-actions">
          <button type="button" class="blender-link" :disabled="Boolean(busyAction)" @click="closeBackground(item.pid)">
            {{ t("blender.status.close") }}
          </button>
        </div>
      </div>
    </div>

    <details v-if="otherInstances.length > 0" class="blender-other">
      <summary>{{ t("blender.status.otherInstances", otherInstances.length) }}</summary>
      <div v-for="instance in otherInstances" :key="instance.key" class="blender-row" :title="instanceTitle(instance)">
        <span class="blender-dot" :class="dotClass(instance)" aria-hidden="true" />
        <div class="blender-row-main">
          <div class="blender-row-line">
            <span class="blender-file">{{ fileLabel(instance) }}</span>
            <span class="blender-meta">{{ instanceMeta(instance) }}</span>
            <span v-if="isTarget(instance)" class="blender-tag">{{ t("blender.status.target") }}</span>
          </div>
          <div v-if="instance.otherSessionTarget && !isTarget(instance)" class="blender-sub">
            <span class="blender-subtle">{{ t("blender.status.otherSessionTarget") }}</span>
          </div>
        </div>
        <div class="blender-row-actions">
          <button
            v-if="canTarget"
            type="button"
            class="blender-link"
            :disabled="Boolean(busyAction)"
            @click="toggleTarget(instance)"
          >
            {{ isTarget(instance) ? t("blender.status.clearTarget") : t("blender.status.setTarget") }}
          </button>
        </div>
      </div>
    </details>

    <div class="blender-footer">
      <div v-if="versionText" class="blender-footer-line" :title="versionTitle">
        <span class="blender-footer-label">{{ t("blender.status.workspaceVersion") }}</span>
        <span>{{ versionText }}</span>
      </div>
      <div v-if="addonLine" class="blender-footer-line">
        <span class="blender-attention">{{ addonLine.text }}</span>
        <button
          v-if="addonLine.action"
          type="button"
          class="blender-link is-accent"
          :disabled="Boolean(busyAction)"
          @click="runAddonAction"
        >
          {{ addonLine.action === "reload"
            ? t("blender.status.reload")
            : addon?.status === "missing" ? t("blender.status.install") : t("blender.status.update") }}
        </button>
      </div>
      <div v-if="withoutHost.length > 0" class="blender-footer-line">
        {{ t("blender.status.withoutHost", withoutHost.length) }}
      </div>
      <div v-if="actionError" class="blender-error" role="alert">{{ actionError }}</div>
    </div>
  </div>
</template>

<style scoped>
.blender-status-panel {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 4px;
  font-size: 12px;
}

.blender-list {
  display: flex;
  flex-direction: column;
}

.blender-empty {
  padding: 2px 0;
  color: var(--text-secondary);
  font-size: 11px;
}

.blender-dormant {
  display: flex;
  align-items: center;
  gap: 6px;
}

.blender-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  min-width: 0;
  margin: 0 -6px;
  padding: 4px 6px;
  border-radius: 4px;
}

.blender-row:hover {
  background: color-mix(in srgb, var(--text-color) 5%, transparent);
}

.blender-row-main {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1 1 auto;
}

.blender-row-line {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.blender-file {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
}

.blender-meta {
  /* Gives way long before the file name does. */
  flex: 0 100 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.blender-sub {
  display: flex;
  gap: 6px;
  min-width: 0;
  font-size: 11px;
}

.blender-subtle {
  flex: 0 0 auto;
  color: var(--text-secondary);
}

.blender-note {
  color: var(--status-warn-fg);
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.blender-tag {
  flex: 0 0 auto;
  color: var(--accent-color);
  font-size: 11px;
}

.blender-dot {
  width: 6px;
  height: 6px;
  margin-top: 5px;
  border-radius: 50%;
  flex: 0 0 auto;
  background: var(--text-secondary);
}

.blender-dot.is-ok { background: var(--status-good-fg); }
.blender-dot.is-warning { background: var(--status-warn-fg); }
.blender-dot.is-danger { background: var(--status-danger-fg); }
.blender-dot.is-busy {
  background: var(--accent-color);
  animation: blender-dot-breathe 1.35s ease-in-out infinite;
}

@keyframes blender-dot-breathe {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.35; }
}

.blender-row-actions {
  display: flex;
  flex: 0 0 auto;
  gap: 2px;
}

.blender-link {
  flex: 0 0 auto;
  white-space: nowrap;
  padding: 0 4px;
  border: none;
  border-radius: 3px;
  background: none;
  color: var(--text-secondary);
  font: inherit;
  font-size: 11px;
  line-height: 18px;
  cursor: pointer;
}

.blender-link:hover:not(:disabled) {
  color: var(--text-color);
  background: color-mix(in srgb, var(--text-color) 8%, transparent);
}

.blender-link.is-accent {
  color: var(--accent-color);
}

.blender-link:disabled {
  opacity: 0.5;
  cursor: default;
}

.blender-other summary {
  cursor: pointer;
  color: var(--text-secondary);
  font-size: 11px;
}

.blender-footer {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-top: 6px;
  border-top: 1px solid color-mix(in srgb, var(--text-color) 8%, transparent);
  color: var(--text-secondary);
  font-size: 11px;
}

.blender-footer-line {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 18px;
}

.blender-attention {
  color: var(--text-color);
}

.blender-error {
  color: var(--status-danger-fg);
}
</style>
