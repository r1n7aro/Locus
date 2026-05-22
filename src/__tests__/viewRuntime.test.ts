import { describe, expect, it } from "vitest";
import { compile } from "vue";
import {
  extractVueScriptSetup,
  transformModuleSource,
  transformViewScriptSetup,
  ViewCompileError,
} from "../components/view/viewCompiler";

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
});
