<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import type { EditorView, ViewUpdate } from "@codemirror/view";
import { X } from "lucide";
import BaseButton from "../BaseButton.vue";
import BaseContextMenu from "../BaseContextMenu.vue";
import { clampFloatingPosition } from "../floatingPosition";
import LucideIcon from "../../icons/LucideIcon.vue";
import type { WorkspaceRef } from "../../../services/project";
import { mapMarkdownEditTarget, serializeMarkdownEdit, type MarkdownEditTarget } from "./markdownEditTarget";
import { searchMarkdownEditorResources, type MarkdownResourceOption } from "./markdownEditorResources";

const props = defineProps<{ view: EditorView; workspaceRef?: WorkspaceRef | null }>();
const owner = `markdown-controls-${Math.random().toString(36).slice(2)}`;
const target = shallowRef<MarkdownEditTarget | null>(null);
const url = ref("");
const label = ref("");
const error = ref("");
const point = ref({ x: 0, y: 0 });
const query = ref("");
const resources = ref<MarkdownResourceOption[]>([]);
const searching = ref(false);
const searchOpen = ref(false);
const highlighted = ref(0);
let searchSequence = 0;
let searchTimer: ReturnType<typeof setTimeout> | undefined;
let disposed = false;
const title = computed(() => target.value?.kind === "image" ? "图片" : target.value?.kind === "reference" ? "引用" : "链接");

function close(focus = false) {
  target.value = null;
  searchOpen.value = false;
  resources.value = [];
  error.value = "";
  searchSequence++;
  clearTimeout(searchTimer);
  if (focus) props.view.focus();
}

function positionAt(position: number) {
  const rect = props.view.coordsAtPos(Math.min(props.view.state.doc.length, position));
  if (!rect) return;
  const popup = document.querySelector<HTMLElement>(`[data-md-owner="${owner}"]`);
  point.value = clampFloatingPosition(
    { x: rect.left, y: rect.bottom + 6 },
    { width: Math.max(300, popup?.offsetWidth ?? 0), height: popup?.offsetHeight ?? 0 },
    { width: window.innerWidth, height: window.innerHeight },
  );
}

function open(next: MarkdownEditTarget) {
  if (props.view.state.readOnly) return;
  close();
  target.value = next;
  url.value = next.url;
  label.value = next.label;
  positionAt(next.from);
}

function refresh() {
  if (disposed || props.view.state.readOnly) return;
  if (target.value) positionAt(target.value.from);
}

function update(update: ViewUpdate) {
  if (target.value && update.docChanged) {
    target.value = mapMarkdownEditTarget(target.value, update.changes);
    if (!target.value) close();
  }
  if (target.value && (update.docChanged || update.viewportChanged || update.geometryChanged)) queueMicrotask(refresh);
}

function apply() {
  const current = target.value;
  if (!current || props.view.state.readOnly) return;
  if (props.view.state.sliceDoc(current.from, current.to) !== current.source) { close(); return; }
  try {
    const insert = serializeMarkdownEdit(current, url.value, label.value);
    close();
    if (insert !== current.source) props.view.dispatch({ changes: { from: current.from, to: current.to, insert }, userEvent: "input.property" });
    props.view.focus();
  } catch (reason) { error.value = reason instanceof Error ? reason.message : "修改失败"; }
}

async function chooseFile() {
  const current = target.value;
  try {
    const { open: selectFile } = await import("@tauri-apps/plugin-dialog");
    const selected = await selectFile({ multiple: false, filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"] }] });
    if (target.value === current && typeof selected === "string") url.value = selected.replace(/\\/g, "/");
  } catch { if (target.value === current) error.value = "无法选择文件"; }
}

async function search() {
  const current = target.value;
  const workspaceRef = props.workspaceRef ? { ...props.workspaceRef } : null;
  const sequence = ++searchSequence;
  if (!current || !workspaceRef) return;
  searching.value = true;
  error.value = "";
  try {
    const result = await searchMarkdownEditorResources(query.value, current, workspaceRef);
    if (disposed || sequence !== searchSequence || target.value !== current) return;
    resources.value = result;
    highlighted.value = 0;
  } catch { if (sequence === searchSequence) error.value = "资源搜索失败"; }
  finally { if (sequence === searchSequence) searching.value = false; }
}

function startSearch() {
  searchOpen.value = true;
  query.value = "";
  void search();
  void nextTick(() => document.querySelector<HTMLInputElement>(`[data-md-owner="${owner}"] .md-resource-query`)?.focus());
}
watch(query, () => { clearTimeout(searchTimer); searchSequence++; searchTimer = setTimeout(() => void search(), 150); });
watch(() => `${props.workspaceRef?.checkoutId}:${props.workspaceRef?.expectedGeneration}:${props.workspaceRef?.expectedMaterializationEpoch}`, () => close());
function selectResource(resource: MarkdownResourceOption) { url.value = resource.path; searchOpen.value = false; searchSequence++; }
function searchKey(event: KeyboardEvent) {
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    highlighted.value = Math.max(0, Math.min(resources.value.length - 1, highlighted.value + (event.key === "ArrowDown" ? 1 : -1)));
    void nextTick(() => document.querySelector(`[data-md-owner="${owner}"] .md-resource-results [aria-selected="true"]`)?.scrollIntoView({ block: "nearest" }));
  } else if (event.key === "Enter" && resources.value[highlighted.value]) { event.preventDefault(); selectResource(resources.value[highlighted.value]); }
}

function outside(event: PointerEvent) {
  if (event.target instanceof Element && event.target.closest(`[data-md-owner="${owner}"]`)) return;
  close();
}
onMounted(() => { document.addEventListener("pointerdown", outside); window.addEventListener("resize", refresh); });
onBeforeUnmount(() => { disposed = true; close(); document.removeEventListener("pointerdown", outside); window.removeEventListener("resize", refresh); });
defineExpose({ open, update });
</script>

<template>
  <BaseContextMenu v-if="target" :x="point.x" :y="point.y" :min-width="300" :max-width="360" :show-backdrop="false" role="dialog" :aria-label="`编辑${title}`" class="md-edit-properties" :data-md-owner="owner" @close="close(true)">
    <form class="md-edit-form" @submit.prevent="apply">
      <div class="md-edit-title"><span>{{ title }}</span><BaseButton :aria-label="'关闭'" @click="close(true)"><LucideIcon :icon="X" :size="14" /></BaseButton></div>
      <label class="md-edit-field"><span>{{ target.kind === 'reference' ? '目标' : '地址' }}</span><input v-model="url" aria-label="地址" spellcheck="false" /></label>
      <label v-if="target.kind === 'image'" class="md-edit-field"><span>说明</span><input v-model="label" aria-label="图片说明" /></label>
      <div class="md-edit-file-actions">
        <BaseButton v-if="workspaceRef && target.kind !== 'link'" @click="startSearch">选择资源</BaseButton>
        <BaseButton v-if="target.kind === 'image'" @click="chooseFile">选择文件</BaseButton>
      </div>
      <template v-if="searchOpen">
        <input v-model="query" class="md-resource-query" aria-label="搜索资源" placeholder="搜索资源" @keydown="searchKey" />
        <div class="md-resource-results" role="listbox" aria-label="资源">
          <BaseButton v-for="(resource, index) in resources" :key="resource.path" role="option" :aria-selected="index === highlighted" :class="{ 'is-highlighted': index === highlighted }" :title="resource.path" @click="selectResource(resource)"><span>{{ resource.name }}</span><small>{{ resource.path }}</small></BaseButton>
          <span v-if="!resources.length" class="md-edit-status">{{ searching ? '搜索中…' : '没有匹配资源' }}</span>
        </div>
      </template>
      <span v-if="error" class="md-edit-error" role="alert">{{ error }}</span>
      <div class="md-edit-footer"><BaseButton @click="close(true)">取消</BaseButton><BaseButton type="submit">应用</BaseButton></div>
    </form>
  </BaseContextMenu>
</template>

<style scoped>
.md-edit-file-actions, .md-edit-footer, .md-edit-title { display: flex; align-items: center; gap: 4px; }
.md-edit-form { display: flex; flex-direction: column; gap: 10px; padding: 7px; font-family: var(--font-ui); font-size: 12px; }
.md-edit-title { justify-content: space-between; font-weight: 600; }
.md-edit-title :deep(.base-button) { width: 26px; padding: 0; justify-content: center; }
.md-edit-field { display: flex; flex-direction: column; gap: 5px; color: var(--text-secondary); }
.md-edit-field input, .md-resource-query { box-sizing: border-box; width: 100%; min-width: 0; padding: 6px 8px; border: 1px solid var(--border-color); border-radius: 6px; background: var(--bg-color); color: var(--text-color); font: inherit; outline: none; }
.md-edit-field input:focus, .md-resource-query:focus { border-color: var(--accent-color); }
.md-edit-field:first-of-type input { font-family: var(--font-mono-inline); }
.md-edit-file-actions:empty { display: none; }
.md-edit-footer { justify-content: flex-end; border-top: 1px solid var(--border-color); padding-top: 8px; }
.md-edit-footer :deep(.base-button), .md-edit-file-actions :deep(.base-button) { width: auto; }
.md-resource-results { max-height: 200px; overflow: auto; }
.md-resource-results :deep(.base-button) { display: flex; flex-direction: column; align-items: flex-start; gap: 3px; padding: 6px 8px; }
.md-resource-results :deep(.base-button.is-highlighted) { background: var(--hover-bg); }
.md-resource-results small { display: block; max-width: 100%; overflow: hidden; text-overflow: ellipsis; color: var(--text-secondary); font-family: var(--font-mono-inline); font-size: 11px; }
.md-edit-status { display: block; padding: 8px; color: var(--text-secondary); }
.md-edit-error { color: var(--status-error-fg); }
</style>
