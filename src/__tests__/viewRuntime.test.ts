import { describe, expect, it } from "vitest";
import { compile } from "vue";
import * as VueRuntime from "vue";
import {
  extractVueScriptSetup,
  transformModuleSource,
  transformViewScriptSetup,
  ViewCompileError,
} from "../components/view/viewCompiler";
import { compileViewSfc } from "../components/view/viewSfcCompiler";

describe("viewRuntime", () => {
  it("extracts and transforms Vue script setup TypeScript", () => {
    const script = extractVueScriptSetup(`
<template><button @click="refresh">{{ message }}</button></template>
<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import { viewState } from "./store";

type MaterialModel = {
  ok: boolean;
  message: string;
};

export interface FieldBinding {
  id: string;
}

const material = reactive<MaterialModel>({ ok: false, message: "" });
const message = ref("Waiting");

onMounted(refresh);

async function refresh() {
  const value = ($event.target as HTMLInputElement).value;
  message.value = value;
}
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module0 = __import("vue")');
    expect(transformed.code).toContain('const __module1 = __import("./store")');
    expect(transformed.code).toContain("const material = reactive(");
    expect(transformed.code).toContain("async function refresh()");
    expect(transformed.code).not.toContain("MaterialModel");
    expect(transformed.code).not.toContain("FieldBinding");
    expect(transformed.code).not.toContain("HTMLInputElement");
    expect(transformed.introducedNames).toContain("material");
    expect(transformed.introducedNames).toContain("message");
    expect(transformed.introducedNames).toContain("viewState");
    expect(transformed.introducedNames).toContain("refresh");
    expect(transformed.introducedNames).not.toContain("__module0");
    expect(transformed.introducedNames).not.toContain("__module1");
  });

  it("does not expose compiler internals as Vue setup bindings", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
import { ref } from "vue";
import { view } from "@locus/view-runtime";

const visible = ref(view.manifest.id);
const _internal = "private";
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module0 = __import("vue")');
    expect(transformed.code).toContain('const __module1 = __import("@locus/view-runtime")');
    expect(transformed.introducedNames).toContain("visible");
    expect(transformed.introducedNames).toContain("view");
    expect(transformed.introducedNames).not.toContain("__module0");
    expect(transformed.introducedNames).not.toContain("__module1");
    expect(transformed.introducedNames).not.toContain("_internal");
    expect(transformed.introducedNames.every((name) => !/^[$_]/.test(name))).toBe(true);
  });

  it("preserves object literal properties while stripping parameter types", () => {
    const script = extractVueScriptSetup(`
<template><main>{{ state.message }}</main></template>
<script setup lang="ts">
import { reactive } from "vue";

type MaterialInfo = { name: string };
type ApiResponse = { ok: boolean; material: MaterialInfo | null };

const state = reactive({
  loading: false,
  saving: false,
  dirty: false,
  message: "Select a Material asset, then refresh.",
  material: null as MaterialInfo | null,
});

async function callUnity(method: string, args: Record<string, unknown>): Promise<ApiResponse> {
  return { ok: true, material: null };
}

function propertyInputType(property: { type: string }) {
  return property.type === "Float" ? "number" : "text";
}
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain("loading: false");
    expect(transformed.code).toContain("saving: false");
    expect(transformed.code).toContain("dirty: false");
    expect(transformed.code).toContain('message: "Select a Material asset, then refresh."');
    expect(transformed.code).toContain("material: null");
    expect(transformed.code).toContain("async function callUnity(method, args)");
    expect(transformed.code).toContain("function propertyInputType(property)");
    expect(transformed.code).not.toContain("Record<string, unknown>");
    expect(transformed.code).not.toContain("Promise<ApiResponse>");
    expect(transformed.code).not.toContain("MaterialInfo");

    expect(() => {
      const execute = new Function("__import", "window", `${transformed.code}\nreturn state;`);
      execute(
        () => ({ reactive: (value: unknown) => value }),
        { locus: { view: { callScript: async () => ({}) } } },
      );
    }).not.toThrow();
  });

  it("strips generic, array, and object return type annotations", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
type ApiResponse = { ok: boolean };
function read(): Promise<ApiResponse> {
  return Promise.resolve({ ok: true });
}
function values(): string[] {
  return [];
}
const format = (value: string): { text: string } => ({ text: value });
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain("function read()");
    expect(transformed.code).toContain("function values()");
    expect(transformed.code).toContain("const format = (value) =>");
    expect(transformed.code).not.toContain(": Promise");
    expect(transformed.code).not.toContain(": string[]");
    expect(transformed.code).not.toContain(": { text: string }");
    expect(() => new Function(transformed.code)).not.toThrow();
  });

  it("transpiles function type annotations used by view update unsubscribers", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
let unsubscribeUpdate: (() => void) | null = null;

function setUnsubscribe(next: () => void) {
  unsubscribeUpdate = next;
}

setUnsubscribe(() => undefined);
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain("let unsubscribeUpdate = null");
    expect(transformed.code).toContain("function setUnsubscribe(next)");
    expect(transformed.code).not.toContain("=> void");
    expect(() => new Function(transformed.code)).not.toThrow();
  });

  it("reports TypeScript syntax diagnostics from the View compiler", () => {
    let error: unknown;

    try {
      transformViewScriptSetup("const broken: = 1;", "src/App.vue");
    } catch (compileError) {
      error = compileError;
    }

    expect(error).toBeInstanceOf(ViewCompileError);
    const compileError = error as ViewCompileError;
    expect(compileError.diagnostics[0]).toMatchObject({
      category: "error",
      code: 1110,
      line: 1,
    });
    expect(compileError.message).toContain("src/App.vue?script-setup.ts:1");
  });

  it("transforms TypeScript module exports through the compiler layer", () => {
    const code = transformModuleSource(`
export class Counter {
  value: number = 1;
}

export enum CounterMode {
  Single,
}

export const answer: number = 42;
export { answer as alias };
`, "src/counter.ts");

    expect(code).toContain("const require = __import");
    expect(code).toContain("exports.alias = exports.answer = exports.CounterMode = exports.Counter = void 0");
    expect(code).toContain("exports.Counter = Counter");
    expect(code).toContain("exports.CounterMode = CounterMode = {}");
    expect(code).toContain("exports.answer = 42");
    expect(code).toContain("exports.alias = exports.answer");

    expect(() => {
      const module = { exports: {} as Record<string, unknown> };
      const execute = new Function("__import", "exports", "module", code);
      execute(() => ({}), module.exports, module);
    }).not.toThrow();
  });

  it("uses the Vue build with runtime template compilation", () => {
    const render = compile("<main>{{ message }}</main>");

    expect(typeof render).toBe("function");
  });

  it("compiles View SFCs through the official Vue compiler", () => {
    const compiled = compileViewSfc(`
<script setup lang="ts">
const props = withDefaults(defineProps<{ label?: string }>(), {
  label: "Ready",
});
const emit = defineEmits<{ save: [value: string] }>();

function save() {
  emit("save", props.label);
}
</script>

<template>
  <button class="title" type="button" @click="save">
    {{ props.label }}
    <template v-if="props.label">
      <span>active</span>
    </template>
  </button>
</template>

<style scoped>
.title {
  color: red;
}
</style>`, "src/App.vue");
    const module = { exports: {} as Record<string, unknown> };
    const execute = new Function("__import", "exports", "module", "__vue", "__runtime", compiled.code);

    execute(
      (specifier: string) => {
        if (specifier === "vue") return VueRuntime;
        throw new Error(`Unexpected import: ${specifier}`);
      },
      module.exports,
      module,
      VueRuntime,
      {},
    );

    const component = module.exports.default as {
      props?: Record<string, { default?: string }>;
      render?: unknown;
      __scopeId?: string;
    };
    expect(component.props?.label.default).toBe("Ready");
    expect(typeof component.render).toBe("function");
    expect(component.__scopeId).toBe(compiled.scopeId);
    expect(compiled.styles[0]).toContain(`[${compiled.scopeId}]`);
  });

  it("exposes Unity editor update handlers through the View runtime", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
import { onMounted } from "vue";
import { onEditorUpdate, view } from "@locus/view-runtime";

type EditorUpdate = { sequence: number };

onMounted(async () => {
  await onEditorUpdate((event: EditorUpdate) => {
    view.manifest.id;
    event.sequence;
  });
});
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module1 = __import("@locus/view-runtime")');
    expect(transformed.code).toContain("const { onEditorUpdate, view } = __module1");
    expect(transformed.code).toContain("await onEditorUpdate((event) =>");
    expect(transformed.introducedNames).toContain("onEditorUpdate");
    expect(transformed.introducedNames).toContain("view");
  });

  it("exposes LLM and session helpers through the View runtime", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
import { llm, session, view } from "@locus/view-runtime";

async function rewriteGraph(graph: unknown) {
  const createdSessionId = await session.create({ title: "Shader Graph", sessionType: "view" });
  await session.show(createdSessionId);
  const response = await llm.call({
    sessionId: createdSessionId,
    prompt: JSON.stringify(graph),
  });
  view.session.display(response.sessionId);
  return response.text;
}
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module0 = __import("@locus/view-runtime")');
    expect(transformed.code).toContain("const { llm, session, view } = __module0");
    expect(transformed.code).toContain("async function rewriteGraph(graph)");
    expect(transformed.introducedNames).toContain("llm");
    expect(transformed.introducedNames).toContain("session");
    expect(transformed.introducedNames).toContain("view");
    expect(transformed.introducedNames).toContain("rewriteGraph");
  });

  it("supports graph template controllers from the View runtime", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
import { GraphView, GraphViewController, defineGraphView } from "@locus/view-runtime";

class TemplateGraphView extends GraphViewController {
  loadGraph() {
    return {
      nodes: [
        { id: "a", title: "A", outputs: [{ id: "out", label: "Out" }] },
        { id: "b", title: "B", inputs: [{ id: "in", label: "In" }] }
      ],
      connections: [
        { from: { nodeId: "a", portId: "out" }, to: { nodeId: "b", portId: "in" } }
      ]
    };
  }
}

const graphView = defineGraphView(new TemplateGraphView());
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module0 = __import("@locus/view-runtime")');
    expect(transformed.code).toContain("const { GraphView, GraphViewController, defineGraphView } = __module0");
    expect(transformed.code).toContain("class TemplateGraphView extends GraphViewController");
    expect(transformed.code).toContain("const graphView = defineGraphView(new TemplateGraphView())");
    expect(transformed.introducedNames).toContain("GraphView");
    expect(transformed.introducedNames).toContain("GraphViewController");
    expect(transformed.introducedNames).toContain("defineGraphView");
    expect(transformed.introducedNames).toContain("TemplateGraphView");
    expect(transformed.introducedNames).toContain("graphView");
  });

  it("exposes the canvas component to View packages", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
import { CanvasView } from "@locus/view-runtime";

const blocks = [{ id: "a", x: 0, y: 0, width: 240, height: 120 }];
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module0 = __import("@locus/view-runtime")');
    expect(transformed.code).toContain("const { CanvasView } = __module0");
    expect(transformed.introducedNames).toContain("CanvasView");
    expect(transformed.introducedNames).toContain("blocks");
  });

  it("exposes View log helpers to View packages", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
import { view } from "@locus/view-runtime";

async function openLogs() {
  const latest = await view.logs.latest();
  await view.logs.open();
  return latest;
}
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module0 = __import("@locus/view-runtime")');
    expect(transformed.code).toContain("const { view } = __module0");
    expect(transformed.code).toContain("await view.logs.latest()");
    expect(transformed.code).toContain("await view.logs.open()");
    expect(transformed.introducedNames).toContain("view");
    expect(transformed.introducedNames).toContain("openLogs");
  });

  it("exposes Unity property editor components to View packages", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
import { UnityPropertyDraw, UnityPropertyEditor, UnityNumberField, createPropertyTree } from "@locus/components";

const cell = { type: "Integer", value: 1 };
const tree = createPropertyTree({ propertyPath: "m_Name", valueType: "String" });
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module0 = __import("@locus/components")');
    expect(transformed.code).toContain("const { UnityPropertyDraw, UnityPropertyEditor, UnityNumberField, createPropertyTree } = __module0");
    expect(transformed.introducedNames).toContain("UnityPropertyDraw");
    expect(transformed.introducedNames).toContain("UnityPropertyEditor");
    expect(transformed.introducedNames).toContain("UnityNumberField");
    expect(transformed.introducedNames).toContain("createPropertyTree");
    expect(transformed.introducedNames).toContain("cell");
    expect(transformed.introducedNames).toContain("tree");
  });

  it("exposes PropertyTree helpers through the View runtime module", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
import {
  createInspectorPropertyDrawLibrary,
  createPropertyTree,
  projectInspectorPropertyDrawLibrary,
  propertyTreeService,
  publicInspectorPropertyDrawLibrary,
  UnityPropertyDraw,
  view,
} from "@locus/view-runtime";

const tree = createPropertyTree({ propertyPath: "m_Name", valueType: "String" });
const library = createInspectorPropertyDrawLibrary();
const publicLibrary = publicInspectorPropertyDrawLibrary;
const projectLibrary = projectInspectorPropertyDrawLibrary;
const runtimeLibrary = view.propertyDraw.library;
const serviceTree = propertyTreeService.createTree({ propertyPath: "m_Enabled", valueType: "Boolean" });
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module0 = __import("@locus/view-runtime")');
    expect(transformed.code).toContain("const { createInspectorPropertyDrawLibrary, createPropertyTree, projectInspectorPropertyDrawLibrary, propertyTreeService, publicInspectorPropertyDrawLibrary, UnityPropertyDraw, view } = __module0");
    expect(transformed.introducedNames).toContain("createInspectorPropertyDrawLibrary");
    expect(transformed.introducedNames).toContain("createPropertyTree");
    expect(transformed.introducedNames).toContain("projectInspectorPropertyDrawLibrary");
    expect(transformed.introducedNames).toContain("propertyTreeService");
    expect(transformed.introducedNames).toContain("publicInspectorPropertyDrawLibrary");
    expect(transformed.introducedNames).toContain("UnityPropertyDraw");
    expect(transformed.introducedNames).toContain("view");
    expect(transformed.introducedNames).toContain("library");
    expect(transformed.introducedNames).toContain("publicLibrary");
    expect(transformed.introducedNames).toContain("projectLibrary");
    expect(transformed.introducedNames).toContain("runtimeLibrary");
    expect(transformed.introducedNames).toContain("tree");
    expect(transformed.introducedNames).toContain("serviceTree");
  });

  it("exposes View undo and undoable property binding helpers", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
import { undo, useUnityBinding, view } from "@locus/view-runtime";

const nameBinding = useUnityBinding({ target: { kind: "selection", propertyPath: "m_Name" } });

async function rename(commit) {
  await view.binding.writeProperty({ target: { kind: "selection", propertyPath: "m_Name" } }, commit);
  await nameBinding.writeProperty(commit);
  await undo.undo();
  await undo.redo();
}
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module0 = __import("@locus/view-runtime")');
    expect(transformed.code).toContain("const { undo, useUnityBinding, view } = __module0");
    expect(transformed.code).toContain("await view.binding.writeProperty");
    expect(transformed.code).toContain("await nameBinding.writeProperty(commit)");
    expect(transformed.code).toContain("await undo.undo()");
    expect(transformed.code).toContain("await undo.redo()");
    expect(transformed.introducedNames).toContain("undo");
    expect(transformed.introducedNames).toContain("useUnityBinding");
    expect(transformed.introducedNames).toContain("nameBinding");
    expect(transformed.introducedNames).toContain("rename");
  });

  it("exposes Unity selection and drag helpers through the View runtime", () => {
    const script = extractVueScriptSetup(`
<script setup lang="ts">
import {
  UnityDropZone,
  UnityReferenceChip,
  unity,
  useLocusFileDrag,
  useLocusFileDropTarget,
  useUnityAssetDropTarget,
  useUnityReferenceDrag,
} from "@locus/view-runtime";

const materialRef = { kind: "asset", path: "Assets/Materials/M_Wood.mat", name: "M_Wood" };
const refDrag = useUnityReferenceDrag(() => [materialRef]);
const fileDrag = useLocusFileDrag({ path: "Assets/Materials/M_Wood.mat", isDir: false });
const assetDrop = useUnityAssetDropTarget({ onDrop: refs => refs.length });
const fileDrop = useLocusFileDropTarget();

async function openMaterial() {
  await unity.select(materialRef);
  await unity.inspect(materialRef);
  await unity.selectAsset(materialRef.path);
  await unity.inspectSceneObject("Assets/Scenes/Main.unity", "Player/Camera");
  unity.drag.onDrop(() => undefined);
  unity.drag.onState(() => undefined);
  refDrag.draggable.value;
  fileDrag.draggable.value;
  assetDrop.active.value;
  fileDrop.active.value;
  return [UnityDropZone, UnityReferenceChip];
}
</script>`);

    const transformed = transformViewScriptSetup(script);

    expect(transformed.code).toContain('const __module0 = __import("@locus/view-runtime")');
    expect(transformed.code).toContain("const { UnityDropZone, UnityReferenceChip, unity, useLocusFileDrag, useLocusFileDropTarget, useUnityAssetDropTarget, useUnityReferenceDrag } = __module0");
    expect(transformed.code).toContain("const refDrag = useUnityReferenceDrag(() => [materialRef])");
    expect(transformed.code).toContain("const assetDrop = useUnityAssetDropTarget({ onDrop: refs => refs.length })");
    expect(transformed.code).toContain("await unity.select(materialRef)");
    expect(transformed.code).toContain("await unity.inspect(materialRef)");
    expect(transformed.introducedNames).toContain("unity");
    expect(transformed.introducedNames).toContain("useUnityReferenceDrag");
    expect(transformed.introducedNames).toContain("useUnityAssetDropTarget");
    expect(transformed.introducedNames).toContain("UnityReferenceChip");
    expect(transformed.introducedNames).toContain("UnityDropZone");
  });
});
