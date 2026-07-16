<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import {
  ChevronRight,
  CircleMinus,
  CircleX,
  ExternalLink,
  LoaderCircle,
  Play,
  RefreshCw,
  Search,
  Square,
} from "lucide";
import { t } from "../i18n";
import { useUnityTestDashboard } from "../composables/useUnityTestDashboard";
import type {
  UnityTestAssemblyView,
  UnityTestFixtureView,
  UnityTestModeView,
} from "../utils/unityTestDashboard";
import LucideIcon from "./icons/LucideIcon.vue";
import BaseButton from "./ui/BaseButton.vue";
import BaseCheckbox from "./ui/BaseCheckbox.vue";
import BaseDropdown, { type DropdownOption } from "./ui/BaseDropdown.vue";

const props = defineProps<{ workingDir: string }>();
const dashboard = useUnityTestDashboard(() => props.workingDir);
const runScope = ref("");
const dashboardRef = ref<HTMLElement | null>(null);
const browserPaneWidth = ref<number | null>(null);
let resizeStartX = 0;
let resizeStartWidth = 0;

const modeOptions = computed<DropdownOption[]>(() => [
  { value: "all", label: t("unityTest.filter.modeAll") },
  { value: "editmode", label: "EditMode" },
  { value: "playmode", label: "PlayMode" },
]);
const statusOptions = computed<DropdownOption[]>(() => [
  { value: "all", label: t("unityTest.filter.statusAll") },
  { value: "failed", label: t("unityTest.status.failed") },
  { value: "skipped", label: t("unityTest.status.skipped") },
  { value: "not_run", label: t("unityTest.status.notRun") },
]);
const runScopeOptions = computed<DropdownOption[]>(() => [
  { value: "all", label: t("unityTest.run.all") },
  { value: "editmode", label: t("unityTest.run.allEditMode") },
  { value: "playmode", label: t("unityTest.run.allPlayMode") },
]);

watch(runScope, (scope) => {
  if (scope !== "all" && scope !== "editmode" && scope !== "playmode") return;
  runScope.value = "";
  void dashboard.runBroad(scope);
});

function assemblyKeys(assembly: UnityTestAssemblyView): string[] {
  return [...dashboard.testIndex.value.values()]
    .filter((leaf) => leaf.testMode === assembly.testMode && leaf.assemblyName === assembly.name)
    .map((leaf) => leaf.key);
}

function modeKeys(mode: UnityTestModeView): string[] {
  return [...dashboard.testIndex.value.values()]
    .filter((leaf) => leaf.testMode === mode.testMode)
    .map((leaf) => leaf.key);
}

function fixtureKeys(assembly: UnityTestAssemblyView, fixture: UnityTestFixtureView): string[] {
  return [...dashboard.testIndex.value.values()]
    .filter((leaf) => leaf.testMode === assembly.testMode
      && leaf.assemblyName === assembly.name
      && leaf.fixtureName === fixture.name)
    .map((leaf) => leaf.key);
}

function setBranch(keys: string[]) {
  dashboard.setBranchChecked(keys, dashboard.branchState(keys) !== "all");
}

function outcomeLabel(outcome?: string): string {
  const normalized = outcome?.toLowerCase();
  if (normalized === "passed") return t("unityTest.status.passed");
  if (normalized === "failed") return t("unityTest.status.failed");
  if (normalized === "skipped") return t("unityTest.status.skipped");
  return t("unityTest.status.notRun");
}

function formatDuration(seconds?: number): string {
  if (seconds == null) return "-";
  if (seconds < 1) return `${Math.round(seconds * 1000)} ms`;
  return `${seconds.toFixed(2)} s`;
}

function formatDate(value?: string): string {
  if (!value) return "-";
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

function terminalStatusLabel(status: string): string {
  switch (status.toLowerCase()) {
    case "completed": return t("unityTest.terminal.completed");
    case "completed_failed": return t("unityTest.terminal.completedFailed");
    case "cancelled": return t("unityTest.terminal.cancelled");
    case "failed": return t("unityTest.terminal.failed");
    default: return status;
  }
}

function onResizeMove(event: MouseEvent) {
  const containerWidth = dashboardRef.value?.getBoundingClientRect().width ?? window.innerWidth;
  const maxWidth = Math.max(420, containerWidth - 360);
  browserPaneWidth.value = Math.min(maxWidth, Math.max(300, resizeStartWidth + event.clientX - resizeStartX));
}

function onResizeEnd() {
  document.removeEventListener("mousemove", onResizeMove);
  document.removeEventListener("mouseup", onResizeEnd);
  document.body.style.cursor = "";
  document.body.style.userSelect = "";
}

function onResizeStart(event: MouseEvent) {
  resizeStartX = event.clientX;
  resizeStartWidth = dashboardRef.value?.querySelector<HTMLElement>(".browser-pane")?.getBoundingClientRect().width ?? 420;
  document.addEventListener("mousemove", onResizeMove);
  document.addEventListener("mouseup", onResizeEnd);
  document.body.style.cursor = "col-resize";
  document.body.style.userSelect = "none";
}

onUnmounted(onResizeEnd);

const progressPercent = computed(() => {
  const progress = dashboard.activeProgress.value;
  if (!progress?.total) return 0;
  return Math.min(100, Math.round((progress.completed / progress.total) * 100));
});
const latestRunResults = computed(() => dashboard.snapshot.value?.phaseSummaries.flatMap((phase) => phase.results) ?? []);
</script>

<template>
  <main ref="dashboardRef" class="test-dashboard">
    <section
      class="browser-pane"
      :style="{ width: browserPaneWidth == null ? '42%' : `${browserPaneWidth}px` }"
      aria-labelledby="unity-test-browser-title"
    >
      <header class="pane-header">
        <div>
          <h1 id="unity-test-browser-title">{{ t("unityTest.title") }}</h1>
          <span>{{ dashboard.totalTests.value }} {{ t("unityTest.tests") }}</span>
        </div>
        <BaseButton
          :title="t('unityTest.refresh')"
          :disabled="dashboard.loading.value || !workingDir"
          :aria-label="t('unityTest.refresh')"
          @click="dashboard.refresh"
        >
          <LucideIcon :icon="RefreshCw" :size="13" :class="{ spinning: dashboard.loading.value }" />
        </BaseButton>
      </header>

      <div class="filters">
        <label class="search-field">
          <LucideIcon :icon="Search" :size="14" />
          <input v-model="dashboard.search.value" :placeholder="t('unityTest.search')" />
        </label>
        <BaseDropdown
          v-model="dashboard.modeFilter.value"
          :options="modeOptions"
          menu-align="start"
          :aria-label="t('unityTest.filter.mode')"
        />
        <BaseDropdown
          v-model="dashboard.statusFilter.value"
          :options="statusOptions"
          menu-align="start"
          :aria-label="t('unityTest.filter.status')"
        />
      </div>

      <div class="tree-scroll" role="tree" :aria-busy="dashboard.loading.value">
        <div v-if="!workingDir" class="empty-state">{{ t("unityTest.empty.workspace") }}</div>
        <div v-else-if="dashboard.error.value && !dashboard.discovery.value" class="empty-state error-state">
          <strong>{{ t("unityTest.empty.unavailable") }}</strong>
          <span>{{ dashboard.error.value }}</span>
          <BaseButton @click="dashboard.refresh">{{ t("common.retry") }}</BaseButton>
        </div>
        <div v-else-if="dashboard.loading.value && !dashboard.discovery.value" class="empty-state">
          {{ t("common.loading") }}
        </div>
        <div v-else-if="!dashboard.filteredModeTree.value.length" class="empty-state">
          {{ dashboard.totalTests.value ? t("unityTest.empty.filtered") : t("unityTest.empty.tests") }}
        </div>

        <div v-for="mode in dashboard.filteredModeTree.value" v-else :key="mode.key" class="tree-group">
          <div class="tree-row mode-row" role="treeitem" :aria-expanded="dashboard.expandedKeys.value.has(mode.key)">
            <BaseCheckbox
              :model-value="dashboard.branchState(modeKeys(mode)) === 'all'"
              :indeterminate="dashboard.branchState(modeKeys(mode)) === 'some'"
              :aria-label="mode.name"
              @update:model-value="setBranch(modeKeys(mode))"
            />
            <button class="tree-name branch-name" type="button" @click="dashboard.toggleExpanded(mode.key)">
              <LucideIcon :icon="ChevronRight" :size="13" :class="{ open: dashboard.expandedKeys.value.has(mode.key) }" />
              <span>{{ mode.name }}</span>
            </button>
          </div>

          <div v-if="dashboard.expandedKeys.value.has(mode.key)" role="group">
            <div v-for="assembly in mode.assemblies" :key="assembly.key" class="assembly-group">
              <div class="tree-row assembly-row" role="treeitem" :aria-expanded="dashboard.expandedKeys.value.has(assembly.key)">
                <BaseCheckbox
                  :model-value="dashboard.branchState(assemblyKeys(assembly)) === 'all'"
                  :indeterminate="dashboard.branchState(assemblyKeys(assembly)) === 'some'"
                  :aria-label="assembly.name"
                  @update:model-value="setBranch(assemblyKeys(assembly))"
                />
                <button class="tree-name branch-name" type="button" @click="dashboard.toggleExpanded(assembly.key)">
                  <LucideIcon :icon="ChevronRight" :size="12" :class="{ open: dashboard.expandedKeys.value.has(assembly.key) }" />
                  <span>{{ assembly.name }}</span>
                </button>
              </div>

              <div v-if="dashboard.expandedKeys.value.has(assembly.key)" role="group">
                <div v-for="fixture in assembly.fixtures" :key="fixture.key" class="fixture-group">
                  <div class="tree-row fixture-row" role="treeitem" :aria-expanded="dashboard.expandedKeys.value.has(fixture.key)">
                    <BaseCheckbox
                      :model-value="dashboard.branchState(fixtureKeys(assembly, fixture)) === 'all'"
                      :indeterminate="dashboard.branchState(fixtureKeys(assembly, fixture)) === 'some'"
                      :aria-label="fixture.name"
                      @update:model-value="setBranch(fixtureKeys(assembly, fixture))"
                    />
                    <button class="tree-name branch-name" type="button" @click="dashboard.toggleExpanded(fixture.key)">
                      <LucideIcon :icon="ChevronRight" :size="12" :class="{ open: dashboard.expandedKeys.value.has(fixture.key) }" />
                      <span>{{ fixture.name }}</span>
                    </button>
                  </div>

                  <div v-if="dashboard.expandedKeys.value.has(fixture.key)" role="group">
                    <div
                      v-for="leaf in fixture.tests"
                      :key="leaf.key"
                      class="tree-row test-row"
                      :class="{ inspected: dashboard.inspectedKey.value === leaf.key }"
                      role="treeitem"
                    >
                      <BaseCheckbox
                        :model-value="dashboard.checkedKeys.value.has(leaf.key)"
                        :aria-label="leaf.test.fullName || leaf.test.name"
                        @update:model-value="dashboard.toggleChecked(leaf.key)"
                      />
                      <button class="tree-name test-name" type="button" @click="dashboard.inspect(leaf.key)">
                        <span class="status-dot" :class="dashboard.resultByKey.value.get(leaf.key)?.outcome?.toLowerCase() || 'not-run'"></span>
                        <span>{{ leaf.test.name }}</span>
                        <small>{{ outcomeLabel(dashboard.resultByKey.value.get(leaf.key)?.outcome) }}</small>
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <footer class="run-bar">
        <template v-if="dashboard.isActive.value">
          <div class="run-inline-status">
            <span class="status-dot running"></span>
            <strong>{{ dashboard.activeProgress.value?.phase || t("unityTest.progress.preparing") }}</strong>
            <small>{{ dashboard.activeProgress.value?.completed ?? 0 }} / {{ dashboard.activeProgress.value?.total ?? 0 }}</small>
          </div>
        </template>
        <template v-else>
          <BaseButton
            variant="primary"
            block
            :disabled="!dashboard.checkedTests.value.length"
            @click="dashboard.runSelected"
          >
            <LucideIcon :icon="Play" :size="13" />
            {{ t("unityTest.run.selected", dashboard.checkedTests.value.length) }}
          </BaseButton>
          <BaseDropdown
            v-model="runScope"
            class="scope-menu"
            :options="runScopeOptions"
            :placeholder="t('unityTest.run.scope')"
            :disabled="!workingDir"
            :aria-label="t('unityTest.run.scope')"
          />
        </template>
      </footer>
    </section>

    <div
      class="pane-resize-handle"
      role="separator"
      aria-orientation="vertical"
      :aria-label="t('unityTest.resizeBrowser')"
      @mousedown="onResizeStart"
    ></div>

    <section class="details-pane" aria-live="polite">
      <template v-if="dashboard.isActive.value">
        <header class="detail-header">
          <div>
            <span class="eyebrow">{{ dashboard.runSource.value === "agent" ? t("unityTest.source.agent") : t("unityTest.source.dashboard") }}</span>
            <h2>{{ t("unityTest.running") }}</h2>
          </div>
          <BaseButton variant="danger" :disabled="dashboard.cancelling.value" @click="dashboard.cancel">
            <LucideIcon :icon="Square" :size="12" />
            {{ dashboard.cancelling.value ? t("unityTest.stopping") : t("unityTest.stop") }}
          </BaseButton>
        </header>
        <div class="progress-view">
          <div class="running-card">
            <div class="running-spinner" aria-hidden="true">
              <LucideIcon :icon="LoaderCircle" :size="20" />
            </div>
            <strong class="running-title">{{ t("unityTest.running") }}</strong>
            <code class="running-test-name">{{ dashboard.activeProgress.value?.currentTest || t("unityTest.progress.preparing") }}</code>
            <progress class="progress-track" :value="progressPercent" max="100"></progress>
            <div class="running-stats">
              <span>{{ t("unityTest.progress.completed", dashboard.activeProgress.value?.completed ?? 0, dashboard.activeProgress.value?.total ?? 0) }}</span>
              <span>{{ t("unityTest.progress.failedCount", dashboard.activeProgress.value?.failed ?? 0) }}</span>
              <span>{{ dashboard.activeProgress.value?.phase || t("unityTest.progress.preparing") }}</span>
            </div>
          </div>
        </div>
      </template>

      <template v-else>
        <div v-if="dashboard.error.value" class="runtime-error-banner" role="alert">
          {{ dashboard.error.value }}
        </div>
        <nav class="detail-tabs" :aria-label="t('unityTest.details')">
          <button :class="{ active: dashboard.detailTab.value === 'latest' }" @click="dashboard.detailTab.value = 'latest'">
            {{ t("unityTest.latest") }}
          </button>
          <button :class="{ active: dashboard.detailTab.value === 'detail' }" @click="dashboard.detailTab.value = 'detail'">
            {{ t("unityTest.testDetail") }}
          </button>
        </nav>

        <div v-if="dashboard.detailTab.value === 'latest'" class="detail-scroll">
          <div v-if="!dashboard.snapshot.value" class="empty-state">{{ t("unityTest.empty.snapshot") }}</div>
          <template v-else>
            <header class="run-summary-header">
              <div>
                <span class="eyebrow">{{ formatDate(dashboard.snapshot.value.finishedAt) }}</span>
                <h2>{{ terminalStatusLabel(dashboard.snapshot.value.terminalStatus) }}</h2>
              </div>
              <strong>{{ formatDuration(dashboard.snapshot.value.totalSummary.duration) }}</strong>
            </header>
            <div class="summary-grid">
              <div><span>{{ t("unityTest.summary.total") }}</span><strong>{{ dashboard.snapshot.value.totalSummary.total }}</strong></div>
              <div class="passed"><span>{{ t("unityTest.status.passed") }}</span><strong>{{ dashboard.snapshot.value.totalSummary.passed }}</strong></div>
              <div class="failed"><span>{{ t("unityTest.status.failed") }}</span><strong>{{ dashboard.snapshot.value.totalSummary.failed }}</strong></div>
              <div><span>{{ t("unityTest.status.skipped") }}</span><strong>{{ dashboard.snapshot.value.totalSummary.skipped }}</strong></div>
            </div>
            <section class="detail-section">
              <h3>{{ t("unityTest.preparation") }}</h3>
              <p>{{ dashboard.snapshot.value.preparation.method }} · {{ dashboard.snapshot.value.preparation.status }}</p>
              <p v-if="dashboard.snapshot.value.preparation.message" class="muted">{{ dashboard.snapshot.value.preparation.message }}</p>
            </section>
            <section v-if="dashboard.snapshot.value.phaseSummaries.length" class="detail-section">
              <h3>{{ t("unityTest.phases") }}</h3>
              <div v-for="phase in dashboard.snapshot.value.phaseSummaries" :key="`${phase.runId}-${phase.testMode}`" class="phase-row">
                <strong>{{ phase.testMode }}</strong>
                <span>{{ phase.passed }} / {{ phase.total }} · {{ formatDuration(phase.duration) }}</span>
              </div>
            </section>
            <section v-if="latestRunResults.some(result => result.outcome !== 'passed')" class="detail-section">
              <h3>{{ t("unityTest.failuresAndSkips") }}</h3>
              <article
                v-for="result in latestRunResults.filter(item => item.outcome !== 'passed')"
                :key="`${result.fixtureName}-${result.fullName}`"
                class="result-item"
                :class="result.outcome"
              >
                <header class="result-item-header">
                  <LucideIcon :icon="result.outcome === 'failed' ? CircleX : CircleMinus" :size="15" />
                  <div>
                    <strong>{{ result.fullName || result.testName }}</strong>
                    <span>{{ result.fixtureName }}</span>
                  </div>
                  <time>{{ formatDuration(result.duration) }}</time>
                </header>
                <div class="result-item-body">
                  <p v-if="result.message" class="result-message">{{ result.message }}</p>
                  <pre v-if="result.stackTrace">{{ result.stackTrace }}</pre>
                </div>
              </article>
            </section>
            <section v-if="dashboard.snapshot.value.error" class="detail-section error-state">
              <h3>{{ dashboard.snapshot.value.error.code }}</h3>
              <p>{{ dashboard.snapshot.value.error.message }}</p>
            </section>
          </template>
        </div>

        <div v-else class="detail-scroll">
          <div v-if="!dashboard.inspectedTest.value" class="empty-state">{{ t("unityTest.empty.detail") }}</div>
          <template v-else>
            <header class="run-summary-header">
              <div>
                <span class="eyebrow">{{ dashboard.inspectedTest.value.fixtureName }}</span>
                <h2>{{ dashboard.inspectedTest.value.test.name }}</h2>
              </div>
              <span class="outcome-pill" :class="dashboard.inspectedResult.value?.outcome || 'not-run'">
                {{ outcomeLabel(dashboard.inspectedResult.value?.outcome) }}
              </span>
            </header>
            <div class="detail-section">
              <dl class="test-metadata">
                <dt>{{ t("unityTest.summary.duration") }}</dt><dd>{{ formatDuration(dashboard.inspectedResult.value?.duration) }}</dd>
                <dt>{{ t("unityTest.mode") }}</dt><dd>{{ dashboard.inspectedTest.value.testMode }}</dd>
                <dt>{{ t("unityTest.fullName") }}</dt><dd>{{ dashboard.inspectedTest.value.test.fullName }}</dd>
              </dl>
              <BaseButton
                :disabled="!(dashboard.inspectedResult.value?.sourcePath || dashboard.inspectedTest.value.test.sourcePath)"
                :title="(dashboard.inspectedResult.value?.sourcePath || dashboard.inspectedTest.value.test.sourcePath) ? t('unityTest.openSource') : t('unityTest.sourceUnavailable')"
                @click="dashboard.openSource(dashboard.inspectedTest.value, dashboard.inspectedResult.value)"
              >
                <LucideIcon :icon="ExternalLink" :size="13" />
                {{ t("unityTest.openSource") }}
              </BaseButton>
            </div>
            <section v-if="dashboard.inspectedResult.value?.message" class="detail-section">
              <h3>{{ t("unityTest.assertion") }}</h3>
              <p>{{ dashboard.inspectedResult.value.message }}</p>
            </section>
            <section v-if="dashboard.inspectedResult.value?.stackTrace" class="detail-section">
              <h3>{{ t("unityTest.stackTrace") }}</h3>
              <pre>{{ dashboard.inspectedResult.value.stackTrace }}</pre>
            </section>
          </template>
        </div>
      </template>
    </section>
  </main>
</template>

<style scoped>
.test-dashboard { display: flex; width: 100%; height: 100%; min-height: 0; background: var(--bg-color); color: var(--text-color); }
.browser-pane, .details-pane { min-width: 0; min-height: 0; display: flex; flex-direction: column; }
.browser-pane { flex: none; min-width: 300px; background: var(--panel-bg); }
.details-pane { flex: 1; user-select: text; }
.details-pane button, .details-pane [role="button"] { user-select: none; }
.pane-resize-handle { position: relative; z-index: 10; width: 0; flex: none; cursor: col-resize; }
.pane-resize-handle::before { content: ""; position: absolute; inset: 0 -3px; width: 6px; }
.pane-resize-handle::after { content: ""; position: absolute; top: 0; bottom: 0; left: -1px; width: 2px; background: var(--border-color); transition: background 0.15s; }
.pane-resize-handle:hover::after { background: color-mix(in srgb, var(--accent-color) 55%, transparent); }
.pane-header, .detail-header, .run-summary-header { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
.run-summary-header > div { min-width: 0; flex: 1; }
.pane-header { min-height: 58px; padding: 10px 14px; border-bottom: 1px solid var(--border-color); }
h1, h2, h3, p { margin: 0; }
h1 { font-size: 15px; line-height: 1.3; letter-spacing: 0; }
h2 { font-size: 17px; line-height: 1.4; letter-spacing: 0; overflow-wrap: anywhere; }
h3 { font-size: 12px; letter-spacing: 0; color: var(--text-secondary); }
.pane-header span, .eyebrow, .muted { color: var(--text-secondary); font-size: 11px; }
.filters { display: grid; grid-template-columns: minmax(140px, 1fr) 112px 112px; gap: 7px; padding: 9px 12px; border-bottom: 1px solid var(--border-color); }
.search-field { display: flex; align-items: center; gap: 7px; height: 28px; padding: 0 9px; border: 1px solid var(--border-color); border-radius: 6px; background: var(--input-bg); color: var(--text-secondary); }
.search-field:focus-within { border-color: var(--accent-color); }
.search-field input { min-width: 0; width: 100%; border: 0; outline: 0; background: transparent; color: var(--text-color); font-size: 12px; }
.tree-scroll, .detail-scroll { flex: 1; min-height: 0; overflow: auto; }
.tree-scroll { padding: 6px; }
.tree-row { display: flex; align-items: center; gap: 5px; min-height: 29px; border-radius: 5px; }
.tree-row:hover, .test-row.inspected { background: var(--hover-bg); }
.assembly-row { padding-left: 18px; }
.fixture-row { padding-left: 38px; }
.test-row { padding-left: 58px; }
.tree-name { display: flex; align-items: center; gap: 6px; min-width: 0; flex: 1; height: 28px; padding: 0 7px 0 0; border: 0; background: transparent; color: var(--text-color); text-align: left; cursor: pointer; }
.tree-name > span:not(.status-dot) { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
.tree-name small { margin-left: auto; flex: none; color: var(--text-secondary); font-size: 10px; }
.branch-name :deep(svg) { transition: transform 0.12s ease; }
.branch-name :deep(svg.open) { transform: rotate(90deg); }
.status-dot { flex: none; width: 7px; height: 7px; border-radius: 50%; background: var(--text-secondary); }
.status-dot.passed { background: #2ca66f; }
.status-dot.failed { background: var(--status-danger-fg); }
.summary-grid .passed strong { color: #2ca66f; }
.summary-grid .failed strong { color: var(--status-danger-fg); }
.status-dot.skipped { background: #c79434; }
.status-dot.running { background: var(--accent-color); box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-color) 12%, transparent); }
.run-bar { display: flex; gap: 7px; min-height: 48px; padding: 9px 12px; border-top: 1px solid var(--border-color); }
.run-inline-status { display: flex; align-items: center; gap: 9px; min-width: 0; width: 100%; padding: 0 6px; }
.run-inline-status strong { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
.run-inline-status small { color: var(--text-secondary); font-size: 10px; font-variant-numeric: tabular-nums; }
.scope-menu { width: 132px; flex: none; }
.detail-tabs { display: flex; gap: 4px; min-height: 48px; padding: 9px 16px 0; border-bottom: 1px solid var(--border-color); }
.runtime-error-banner { margin: 10px 14px 0; padding: 8px 10px; border: 1px solid var(--status-danger-border); border-radius: 6px; background: var(--status-danger-bg); color: var(--status-danger-fg); font-size: 11px; overflow-wrap: anywhere; }
.detail-tabs button { padding: 0 12px; border: 0; border-bottom: 2px solid transparent; background: transparent; color: var(--text-secondary); cursor: pointer; font-size: 12px; }
.detail-tabs button.active { border-color: var(--accent-color); color: var(--text-color); }
.detail-scroll, .progress-view { padding: 18px; }
.run-summary-header, .detail-header { padding-bottom: 16px; border-bottom: 1px solid var(--border-color); }
.detail-header { min-height: 74px; padding: 14px 18px; }
.summary-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); border-bottom: 1px solid var(--border-color); }
.summary-grid > div { display: flex; flex-direction: column; gap: 4px; padding: 14px 10px; }
.summary-grid span { font-size: 11px; color: var(--text-secondary); }
.summary-grid strong { font-size: 20px; }
.detail-section { display: grid; gap: 9px; padding: 16px 0; border-bottom: 1px solid var(--border-color); }
.phase-row { display: flex; justify-content: space-between; gap: 12px; font-size: 12px; }
.result-item { overflow: hidden; border: 1px solid var(--border-color); border-left: 3px solid #c79434; border-radius: 5px; background: var(--panel-bg); }
.result-item.failed { border-left-color: var(--status-danger-fg); }
.result-item-header { display: flex; align-items: center; gap: 9px; min-width: 0; padding: 10px 12px; background: color-mix(in srgb, #c79434 9%, var(--panel-bg)); color: #9a6a13; }
.result-item.failed .result-item-header { background: var(--status-danger-bg); color: var(--status-danger-fg); }
.result-item-header > div { display: grid; gap: 2px; min-width: 0; flex: 1; }
.result-item-header strong { overflow: hidden; color: var(--text-color); text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
.result-item-header span, .result-item-header time { color: var(--text-secondary); font-size: 10px; }
.result-item-body { display: grid; gap: 10px; padding: 11px 12px; }
.result-message { padding: 8px 10px; border-left: 3px solid var(--status-danger-fg); background: var(--status-danger-bg); color: var(--status-danger-fg); line-height: 1.5; }
pre, code { font-family: var(--font-mono-block); white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
pre { max-height: 260px; overflow: auto; margin: 0; padding: 10px; background: var(--input-bg); font-size: 11px; line-height: 1.55; }
.empty-state { min-height: 180px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; padding: 24px; color: var(--text-secondary); text-align: center; font-size: 12px; }
.error-state { color: var(--status-danger-fg); }
.progress-view { display: grid; flex: 1; place-items: center; }
.running-card { width: min(520px, 100%); padding: 24px; text-align: center; }
.running-spinner { display: grid; place-items: center; width: 40px; height: 40px; margin: 0 auto 12px; border-radius: 50%; background: color-mix(in srgb, var(--accent-color) 12%, transparent); color: var(--accent-color); }
.running-spinner :deep(svg) { animation: spin 1.1s linear infinite; }
.running-title { display: block; font-size: 13px; }
.running-test-name { display: block; min-height: 18px; margin: 8px 0 16px; color: var(--text-secondary); font-size: 11px; }
.progress-track { width: 100%; height: 5px; overflow: hidden; border: 0; border-radius: 3px; background: var(--border-color); appearance: none; }
.progress-track::-webkit-progress-bar { background: var(--border-color); }
.progress-track::-webkit-progress-value { background: var(--accent-color); transition: width 0.2s ease; }
.progress-track::-moz-progress-bar { background: var(--accent-color); }
.running-stats { display: flex; justify-content: center; gap: 22px; margin-top: 10px; color: var(--text-secondary); font-size: 11px; font-variant-numeric: tabular-nums; }
.outcome-pill { flex: none; padding: 4px 8px; border-radius: 5px; background: var(--hover-bg); font-size: 11px; white-space: nowrap; }
.outcome-pill.failed { color: var(--status-danger-fg); background: var(--status-danger-bg); }
.outcome-pill.passed { color: #2ca66f; }
.test-metadata { display: grid; grid-template-columns: 92px minmax(0, 1fr); gap: 8px 12px; margin: 0; font-size: 12px; }
.test-metadata dt { color: var(--text-secondary); }
.test-metadata dd { min-width: 0; margin: 0; overflow-wrap: anywhere; }
.spinning { animation: spin 0.8s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (max-width: 860px) { .filters { grid-template-columns: 1fr 100px; } .filters > :last-child { grid-column: 1 / -1; } }
</style>
