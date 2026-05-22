import {
  compile,
  computed,
  defineAsyncComponent,
  defineComponent,
  h,
  inject,
  isRef,
  markRaw,
  nextTick,
  onActivated,
  onBeforeUnmount,
  onBeforeMount,
  onBeforeUpdate,
  onDeactivated,
  onErrorCaptured,
  onMounted,
  onUnmounted,
  onUpdated,
  provide,
  reactive,
  readonly,
  ref,
  shallowRef,
  toRef,
  toRefs,
  toRaw,
  unref,
  watch,
  watchEffect,
  type Component,
} from "vue";
import BaseButton from "../ui/BaseButton.vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
import type {
  ViewBindingApplyRequest,
  ViewBindingApplyResult,
  ViewBindingReadRequest,
  ViewBindingReadResult,
  ViewBindingWriteRequest,
  ViewBindingWriteResult,
  ViewCallScriptResult,
  ViewPackageDetail,
  ViewPackageFile,
  ViewRuntimeUpdateEvent,
} from "../../services/view";
import {
  extractVueTemplate,
  sanitizeCssForPreview,
  viewFileContent,
} from "./viewHostPreview";
import {
  extractVueScript,
  extractVueScriptSetup,
  sanitizeTemplateExpressions,
  transformModuleSource,
  transformViewScriptSetup,
} from "./viewCompiler";
import {
  GraphView,
  GraphViewController,
  defineGraphView,
  layoutGraphDocument,
  type GraphConnectionValidation,
  type GraphController,
  type GraphData,
  type GraphEndpoint,
  type GraphLayoutOptions,
  type GraphLink,
  type GraphNode,
  type GraphParameter,
  type GraphParameterOption,
  type GraphParameterType,
  type GraphPort,
  type GraphPortDirection,
} from "../graph";

export {
  extractVueScript,
  extractVueScriptSetup,
  sanitizeTemplateExpressions,
  transformModuleSource,
  transformViewScriptSetup,
  type TransformResult,
  type ViewCompileDiagnostic,
  ViewCompileError,
  compileViewModule,
  formatViewCompileDiagnostics,
  transpileViewTypeScript,
} from "./viewCompiler";
export {
  GraphView,
  GraphViewController,
  defineGraphView,
  layoutGraphDocument,
} from "../graph";

export type {
  GraphConnectionValidation,
  GraphController,
  GraphData,
  GraphEndpoint,
  GraphLayoutOptions,
  GraphLink,
  GraphNode,
  GraphParameter,
  GraphParameterOption,
  GraphParameterType,
  GraphPort,
  GraphPortDirection,
};

type ModuleExports = Record<string, unknown>;
type ViewRuntimeUnsubscribe = () => void;

export interface ViewRuntimeApi {
  callScript(scriptName: string, method: string, args?: unknown): Promise<ViewCallScriptResult>;
  bindingRead(request: Omit<ViewBindingReadRequest, "viewId">): Promise<ViewBindingReadResult>;
  bindingWrite(request: Omit<ViewBindingWriteRequest, "viewId">): Promise<ViewBindingWriteResult>;
  bindingApply(request: Omit<ViewBindingApplyRequest, "viewId">): Promise<ViewBindingApplyResult>;
  onUpdate(handler: (event: ViewRuntimeUpdateEvent) => void): Promise<ViewRuntimeUnsubscribe>;
  reload(): Promise<void>;
}

export interface ViewRuntimeComponentOptions {
  detail: ViewPackageDetail;
  api: ViewRuntimeApi;
}

interface RuntimeContext {
  detail: ViewPackageDetail;
  api: ViewRuntimeApi;
  styles: string[];
  entryComponent?: Component;
  importModule: (specifier: string, importer?: string) => ModuleExports;
}

export type ViewGraphPortDirection = GraphPortDirection;
export type ViewGraphParameterType = GraphParameterType;
export type ViewGraphPort = GraphPort;
export type ViewGraphParameterOption = GraphParameterOption;
export type ViewGraphParameter = GraphParameter;
export type ViewGraphNode = GraphNode;
export type ViewGraphEndpoint = GraphEndpoint;
export type ViewGraphConnection = GraphLink;
export type ViewGraphData = GraphData;
export type ViewGraphConnectionValidation = GraphConnectionValidation;
export type ViewGraphController = GraphController;
const LOCUS_COMPONENTS = {
  BaseButton,
  BaseCheckbox,
  BaseDropdown,
  BaseSegmented,
  BaseSwitch,
  GraphView,
};

function createVueModule(context: RuntimeContext): ModuleExports {
  const createAppShim = (component: Component) => {
    context.entryComponent = component;
    const app: {
      mount: () => undefined;
      use: () => unknown;
      component: () => unknown;
      provide: () => unknown;
    } = {
      mount: () => undefined,
      use: () => app,
      component: () => app,
      provide: () => app,
    };
    return app;
  };

  return {
    compile,
    computed,
    createApp: createAppShim,
    defineAsyncComponent,
    defineComponent,
    h,
    inject,
    isRef,
    markRaw,
    nextTick,
    onActivated,
    onBeforeMount,
    onBeforeUnmount,
    onBeforeUpdate,
    onDeactivated,
    onErrorCaptured,
    onMounted,
    onUnmounted,
    onUpdated,
    provide,
    reactive,
    readonly,
    ref,
    shallowRef,
    toRaw,
    toRef,
    toRefs,
    unref,
    watch,
    watchEffect,
  };
}

function fileByPath(detail: ViewPackageDetail, relPath: string): ViewPackageFile | null {
  return detail.files.find((file) => file.relPath === relPath) ?? null;
}

function normalizeRelPath(value: string): string {
  const parts: string[] = [];
  for (const part of value.replace(/\\/g, "/").split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      parts.pop();
      continue;
    }
    parts.push(part);
  }
  return parts.join("/");
}

function resolveModulePath(specifier: string, importer = "src/App.vue"): string {
  if (!specifier.startsWith(".")) return specifier;
  const base = importer.includes("/") ? importer.slice(0, importer.lastIndexOf("/") + 1) : "";
  const normalized = normalizeRelPath(`${base}${specifier}`);
  const hasExtension = /\.[a-z0-9]+$/i.test(normalized);
  if (hasExtension) return normalized;
  return normalized;
}

function resolveFile(detail: ViewPackageDetail, specifier: string, importer?: string): ViewPackageFile | null {
  const base = resolveModulePath(specifier, importer);
  const candidates = /\.[a-z0-9]+$/i.test(base)
    ? [base]
    : [`${base}.ts`, `${base}.vue`, `${base}.js`, `${base}.css`, `${base}/index.ts`];

  for (const candidate of candidates) {
    const file = fileByPath(detail, candidate);
    if (file) return file;
  }
  return null;
}

function createViewRuntimeApi(detail: ViewPackageDetail, api: ViewRuntimeApi) {
  const view = {
    manifest: readonly(detail.manifest),
    summary: readonly(detail.summary),
    reload: api.reload,
    callScript: async (scriptName: string, method: string, args?: unknown) => {
      const response = await api.callScript(scriptName, method, args);
      return response.result;
    },
    binding: {
      read: (request: Omit<ViewBindingReadRequest, "viewId">) => api.bindingRead(request),
      write: (request: Omit<ViewBindingWriteRequest, "viewId">) => api.bindingWrite(request),
      apply: (request: Omit<ViewBindingApplyRequest, "viewId">) => api.bindingApply(request),
    },
    onUpdate: (handler: (event: ViewRuntimeUpdateEvent) => void) => api.onUpdate(handler),
    readBinding: (bindingId: string, target?: ViewBindingReadRequest["target"]) =>
      api.bindingRead({ bindingId, target }),
    writeBinding: (bindingId: string, value: unknown, target?: ViewBindingWriteRequest["target"]) =>
      api.bindingWrite({ bindingId, value, target }),
    applyBindings: (writes: ViewBindingApplyRequest["writes"]) => api.bindingApply({ writes }),
  };

  return {
    view,
    defineView: <T>(value: T) => value,
    defineGraphView,
    GraphView,
    GraphViewController,
    layoutGraphDocument,
    onEditorUpdate: (handler: (event: ViewRuntimeUpdateEvent) => void) => view.onUpdate(handler),
    useViewState: <T extends object>(initial: T) => reactive(initial),
    useViewScript: (scriptName: string) => ({
      call: (method: string, args?: unknown) => view.callScript(scriptName, method, args),
    }),
    useUnityBinding: (bindingIdOrRequest: string | Omit<ViewBindingReadRequest, "viewId">) => {
      const value = ref<unknown>(null);
      const status = ref("idle");
      const error = ref("");
      const read = async () => {
        status.value = "reading";
        error.value = "";
        try {
          const request =
            typeof bindingIdOrRequest === "string"
              ? { bindingId: bindingIdOrRequest }
              : bindingIdOrRequest;
          const result = await api.bindingRead(request);
          value.value = result.value;
          status.value = "ready";
          return result;
        } catch (readError) {
          status.value = "error";
          error.value = readError instanceof Error ? readError.message : String(readError);
          throw readError;
        }
      };
      const write = async (nextValue = value.value) => {
        status.value = "writing";
        const request =
          typeof bindingIdOrRequest === "string"
            ? { bindingId: bindingIdOrRequest, value: nextValue }
            : { ...bindingIdOrRequest, value: nextValue };
        const result = await api.bindingWrite(request);
        value.value = result.value;
        status.value = "ready";
        return result;
      };
      return { value, status, error, read, write };
    },
  };
}

function installLegacyWindowApi(runtime: ReturnType<typeof createViewRuntimeApi>) {
  const target = window as typeof window & {
    locus?: Record<string, unknown>;
  };
  target.locus = {
    ...(target.locus ?? {}),
    view: runtime.view,
    unity: {
      callScript: runtime.view.callScript,
    },
  };
}

function createModuleLoader(context: RuntimeContext) {
  const cache = new Map<string, ModuleExports>();

  function load(specifier: string, importer = "src/App.vue"): ModuleExports {
    if (specifier === "vue") return createVueModule(context);
    if (specifier === "@locus/view-runtime") return createViewRuntimeApi(context.detail, context.api);
    if (specifier === "@locus/components") return LOCUS_COMPONENTS;

    const file = resolveFile(context.detail, specifier, importer);
    if (!file) {
      throw new Error(`View module not found: ${specifier}`);
    }
    if (cache.has(file.relPath)) return cache.get(file.relPath)!;

    if (file.relPath.endsWith(".css")) {
      context.styles.push(file.content);
      const exports = {};
      cache.set(file.relPath, exports);
      return exports;
    }

    if (file.relPath.endsWith(".vue")) {
      const exports = {
        default: buildSfcComponent(context, file.content, file.relPath),
      };
      cache.set(file.relPath, exports);
      return exports;
    }

    const module = { exports: {} as ModuleExports };
    cache.set(file.relPath, module.exports);
    const code = transformModuleSource(file.content, file.relPath);
    const execute = new Function("__import", "exports", "module", "__vue", "__runtime", code);
    execute(
      (childSpecifier: string) => load(childSpecifier, file.relPath),
      module.exports,
      module,
      createVueModule(context),
      createViewRuntimeApi(context.detail, context.api),
    );
    cache.set(file.relPath, module.exports);
    return module.exports;
  }

  return load;
}

function buildSfcComponent(context: RuntimeContext, source: string, relPath: string): Component {
  const template = sanitizeTemplateExpressions(extractVueTemplate(source));
  const scriptSetup = extractVueScriptSetup(source);
  const script = extractVueScript(source);
  const render = compile(template || "<main />");
  const importModule = context.importModule;

  if (scriptSetup) {
    const transformed = transformViewScriptSetup(scriptSetup, relPath);
    const exposed = transformed.introducedNames;
    return defineComponent({
      name: "LocusViewPackageApp",
      components: LOCUS_COMPONENTS,
      setup() {
        const runtime = createViewRuntimeApi(context.detail, context.api);
        installLegacyWindowApi(runtime);
        const returnObject = exposed.length
          ? `return { ${exposed.map((name) => `${name}: typeof ${name} !== "undefined" ? ${name} : undefined`).join(", ")} };`
          : "return {};";
        const execute = new Function(
          "__import",
          "__vue",
          "__runtime",
          `${transformed.code}\n${returnObject}`,
        );
        return execute(
          (specifier: string) => importModule(specifier, relPath),
          createVueModule(context),
          runtime,
        );
      },
      render,
    });
  }

  if (script) {
    const code = transformModuleSource(script, relPath);
    const module = { exports: {} as ModuleExports };
    const execute = new Function("__import", "exports", "module", "__vue", "__runtime", code);
    execute(
      (specifier: string) => importModule(specifier, relPath),
      module.exports,
      module,
      createVueModule(context),
      createViewRuntimeApi(context.detail, context.api),
    );
    const options = (module.exports.default ?? {}) as Record<string, unknown>;
    return defineComponent({
      ...options,
      components: {
        ...LOCUS_COMPONENTS,
        ...((options.components as Record<string, Component> | undefined) ?? {}),
      },
      render,
    });
  }

  return defineComponent({
    name: "LocusViewPackageApp",
    components: LOCUS_COMPONENTS,
    setup() {
      const runtime = createViewRuntimeApi(context.detail, context.api);
      installLegacyWindowApi(runtime);
      return runtime;
    },
    render,
  });
}

function useViewRuntimeStyles(detail: ViewPackageDetail, styles: string[]) {
  const styleEl = document.createElement("style");
  styleEl.dataset.locusViewRuntimeStyle = detail.manifest.id;
  styleEl.textContent = [
    viewRuntimeBaseCss(),
    sanitizeCssForPreview(viewFileContent(detail, detail.manifest.style)),
    ...styles.map(sanitizeCssForPreview),
  ].join("\n\n");
  document.head.appendChild(styleEl);
  onBeforeUnmount(() => {
    styleEl.remove();
  });
}

function viewRuntimeBaseCss(): string {
  return `body {
  background: var(--bg-color);
  color: var(--text-color);
}

.locus-view-runtime-root {
  min-height: 100vh;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

.view-runtime-error {
  margin: 12px;
  padding: 8px 10px;
  border: 1px solid var(--status-danger-border);
  border-radius: 6px;
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 12px;
  line-height: 1.45;
}`;
}

export function createViewRuntimeComponent(options: ViewRuntimeComponentOptions): Component {
  const styles: string[] = [];
  const context: RuntimeContext = {
    detail: options.detail,
    api: options.api,
    styles,
    importModule: () => {
      throw new Error("View module loader is not ready.");
    },
  };
  context.importModule = createModuleLoader(context);
  const entryExports = context.importModule(options.detail.manifest.entry, "src/App.vue");
  const entryComponent = context.entryComponent
    ?? ((entryExports.default as Component | undefined) || undefined);
  const appFile = fileByPath(options.detail, "src/App.vue");
  const appComponent = entryComponent
    ?? (appFile
      ? buildSfcComponent(context, appFile.content, appFile.relPath)
      : defineComponent({
          setup: () => () => h("main", { class: "view-preview-empty" }, options.detail.manifest.name),
        }));

  return markRaw(
    defineComponent({
      name: "LocusViewRuntimeRoot",
      setup() {
        const runtimeError = ref("");
        useViewRuntimeStyles(options.detail, styles);
        onErrorCaptured((capturedError) => {
          runtimeError.value = capturedError instanceof Error
            ? capturedError.message
            : String(capturedError);
          console.error("[view-runtime]", capturedError);
          return false;
        });
        return () => h("div", { class: "locus-view-runtime-root" }, [
          runtimeError.value
            ? h("div", { class: "view-runtime-error" }, runtimeError.value)
            : h(appComponent),
        ]);
      },
    }),
  );
}
