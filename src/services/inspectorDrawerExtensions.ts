import * as Vue from "vue";
import type { Component } from "vue";
import { listen } from "@tauri-apps/api/event";
import { ipcInvoke } from "./ipc";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import {
  defineInspectorPropertyDrawers,
  pluginInspectorPropertyDrawerLibrary,
  type InspectorPropertyDrawerRegistration,
} from "./propertyTree";
import {
  defineUnityObjectDrawers,
  pluginUnityObjectDrawerLibrary,
  type UnityObjectDrawerRegistration,
} from "./unityObjectDrawer";

/**
 * Loads installed plugin "drawer" packages and registers their inspector
 * property / object drawers into the plugin libraries of the CURRENT window.
 *
 * Every Locus window shares the same SPA entry, so bootstrapping this from
 * main.ts covers chat, the Locus Inspector window, view hosts, and diff
 * review windows alike. Packages are compiled with the same TS/SFC compiler
 * the View runtime uses, but execute against a deliberately small runtime —
 * no fs, session, or llm surface.
 */

export interface PluginDrawerPackageFile {
  relPath: string;
  content: string;
}

export interface PluginDrawerPackage {
  pluginId: string;
  pluginName: string;
  scope: string;
  id: string;
  root: string;
  entry: string;
  files: PluginDrawerPackageFile[];
}

export interface PluginDrawerRuntimeMeta {
  pluginId: string;
  pluginName: string;
  drawerId: string;
}

const PLUGINS_CHANGED_EVENT = "plugins-changed";
const DRAWER_RUNTIME_SPECIFIER = "@locus/drawer-runtime";
const STYLE_DATASET_KEY = "locusPluginDrawerStyles";

type ModuleExports = Record<string, unknown>;
type AnyRegister = (...args: unknown[]) => () => void;

interface DrawerCompilerModule {
  compileViewSfc: (source: string, fileName?: string) => {
    code: string;
    styles: string[];
  };
  transformModuleSource: (source: string, fileName?: string) => string;
}

interface LoadedDrawerPackage {
  key: string;
  disposers: Array<() => void>;
}

let loadRun = 0;
let changeListenerInstalled = false;
let loadedPackages: LoadedDrawerPackage[] = [];
let injectedStyleEl: HTMLStyleElement | null = null;

export function bootstrapPluginInspectorDrawers(): void {
  if (!hasTauriWindowRuntime()) return;
  void reloadPluginInspectorDrawers().catch((error) => {
    console.warn("[inspectorDrawerExtensions] initial drawer load failed:", error);
  });
  if (changeListenerInstalled) return;
  changeListenerInstalled = true;
  void listen(PLUGINS_CHANGED_EVENT, () => {
    void reloadPluginInspectorDrawers().catch((error) => {
      console.warn("[inspectorDrawerExtensions] drawer reload failed:", error);
    });
  }).catch(() => {
    changeListenerInstalled = false;
  });
}

export async function reloadPluginInspectorDrawers(): Promise<void> {
  const run = ++loadRun;
  const packages = await ipcInvoke<PluginDrawerPackage[]>("plugin_inspector_drawer_packages");
  if (run !== loadRun) return;

  unloadPluginInspectorDrawers();
  if (!packages.length) return;

  // The TS/SFC compiler is heavy; only pull it in when drawers exist.
  const [compiler, runtimeModule] = await Promise.all([
    import("../components/view/viewSfcCompiler"),
    import("../components/view/viewRuntime"),
  ]);
  if (run !== loadRun) return;

  const styles: string[] = [];
  for (const pkg of packages) {
    const disposers: Array<() => void> = [];
    try {
      executeDrawerPackage(pkg, compiler, runtimeModule.LOCUS_COMPONENT_MODULE, styles, disposers);
      loadedPackages.push({ key: `${pkg.scope}:${pkg.pluginId}:${pkg.id}`, disposers });
    } catch (error) {
      disposers.forEach((dispose) => safeDispose(dispose));
      console.warn(
        `[inspectorDrawerExtensions] failed to load drawer ${pkg.pluginId}/${pkg.id}:`,
        error,
      );
    }
  }
  if (run !== loadRun) return;
  applyDrawerStyles(styles);
}

export function unloadPluginInspectorDrawers(): void {
  for (const pkg of loadedPackages) {
    pkg.disposers.forEach((dispose) => safeDispose(dispose));
  }
  loadedPackages = [];
  injectedStyleEl?.remove();
  injectedStyleEl = null;
}

function safeDispose(dispose: () => void) {
  try {
    dispose();
  } catch (error) {
    console.warn("[inspectorDrawerExtensions] drawer unregister failed:", error);
  }
}

function applyDrawerStyles(styles: string[]) {
  if (!styles.length || typeof document === "undefined") return;
  const styleEl = document.createElement("style");
  styleEl.dataset[STYLE_DATASET_KEY] = "true";
  styleEl.textContent = styles.join("\n\n");
  document.head.appendChild(styleEl);
  injectedStyleEl = styleEl;
}

function executeDrawerPackage(
  pkg: PluginDrawerPackage,
  compiler: DrawerCompilerModule,
  componentModule: Record<string, unknown>,
  styles: string[],
  disposers: Array<() => void>,
) {
  const files = new Map<string, string>();
  for (const file of pkg.files) {
    files.set(file.relPath.replace(/\\/g, "/"), file.content);
  }
  const runtime = createDrawerRuntime(pkg, disposers, componentModule);
  const vueModule: ModuleExports = { ...Vue };
  const cache = new Map<string, ModuleExports>();
  const compilingSfcPaths = new Set<string>();

  const load = (specifier: string, importer: string): ModuleExports => {
    if (specifier === "vue") return vueModule;
    if (specifier === DRAWER_RUNTIME_SPECIFIER) return runtime as unknown as ModuleExports;
    if (specifier === "@locus/components") return componentModule as ModuleExports;
    if (specifier === "@locus/view-runtime") {
      throw new Error(
        "@locus/view-runtime is not available in drawer packages; import @locus/drawer-runtime instead.",
      );
    }

    const resolved = resolveDrawerModuleFile(files, specifier, importer);
    if (!resolved) {
      throw new Error(`Drawer module not found: ${specifier} (from ${importer})`);
    }
    const { relPath, content } = resolved;
    const cached = cache.get(relPath);
    if (cached) return cached;

    if (relPath.endsWith(".css")) {
      styles.push(content);
      const exports: ModuleExports = {};
      cache.set(relPath, exports);
      return exports;
    }

    if (relPath.endsWith(".json")) {
      const exports: ModuleExports = { default: JSON.parse(content) };
      cache.set(relPath, exports);
      return exports;
    }

    if (relPath.endsWith(".vue")) {
      if (compilingSfcPaths.has(relPath)) {
        const chain = [...compilingSfcPaths, relPath].join(" -> ");
        throw new Error(`Circular import between .vue files is not supported: ${chain}`);
      }
      compilingSfcPaths.add(relPath);
      try {
        const compiled = compiler.compileViewSfc(content, relPath);
        styles.push(...compiled.styles);
        const component = executeCompiledModule(compiled.code, relPath, load, vueModule, runtime);
        const options = (component.default ?? {}) as Record<string, unknown>;
        const exports: ModuleExports = {
          default: Vue.defineComponent({
            ...options,
            components: {
              ...(componentModule as Record<string, Component>),
              ...((options.components as Record<string, Component> | undefined) ?? {}),
            },
          }),
        };
        cache.set(relPath, exports);
        return exports;
      } finally {
        compilingSfcPaths.delete(relPath);
      }
    }

    const code = compiler.transformModuleSource(content, relPath);
    const moduleExports = executeCompiledModule(code, relPath, load, vueModule, runtime);
    cache.set(relPath, moduleExports);
    return moduleExports;
  };

  load(`./${pkg.entry}`, "");
}

function executeCompiledModule(
  code: string,
  relPath: string,
  load: (specifier: string, importer: string) => ModuleExports,
  vueModule: ModuleExports,
  runtime: unknown,
): ModuleExports {
  const module = { exports: {} as ModuleExports };
  const execute = new Function("__import", "exports", "module", "__vue", "__runtime", code);
  execute(
    (specifier: string) => load(specifier, relPath),
    module.exports,
    module,
    vueModule,
    runtime,
  );
  return module.exports;
}

function resolveDrawerModuleFile(
  files: Map<string, string>,
  specifier: string,
  importer: string,
): { relPath: string; content: string } | null {
  if (!specifier.startsWith("./") && !specifier.startsWith("../")) return null;
  const importerDir = importer.includes("/") ? importer.slice(0, importer.lastIndexOf("/")) : "";
  const base = joinPackagePath(importerDir, specifier);
  if (base == null) return null;
  const candidates = [
    base,
    `${base}.ts`,
    `${base}.js`,
    `${base}.vue`,
    `${base}.css`,
    `${base}.json`,
    `${base}/index.ts`,
    `${base}/index.js`,
  ];
  for (const candidate of candidates) {
    const content = files.get(candidate);
    if (content !== undefined) return { relPath: candidate, content };
  }
  return null;
}

function joinPackagePath(baseDir: string, specifier: string): string | null {
  const segments = baseDir ? baseDir.split("/").filter(Boolean) : [];
  for (const part of specifier.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      // Imports must stay inside the drawer package.
      if (!segments.length) return null;
      segments.pop();
      continue;
    }
    segments.push(part);
  }
  return segments.join("/");
}

function createDrawerRuntime(
  pkg: PluginDrawerPackage,
  disposers: Array<() => void>,
  componentModule: Record<string, unknown>,
) {
  const track = (dispose: () => void) => {
    disposers.push(dispose);
    return dispose;
  };
  const registerPropertyDrawer: AnyRegister = (...args) =>
    track((pluginInspectorPropertyDrawerLibrary.register as AnyRegister)(...args));
  const registerObjectDrawer: AnyRegister = (...args) =>
    track((pluginUnityObjectDrawerLibrary.register as AnyRegister)(...args));

  const meta: PluginDrawerRuntimeMeta = {
    pluginId: pkg.pluginId,
    pluginName: pkg.pluginName,
    drawerId: pkg.id,
  };

  return {
    meta,
    components: componentModule,
    propertyDrawer: {
      register: registerPropertyDrawer,
      registerValue: (
        valueType: string | string[],
        drawer: Component,
        options: Omit<InspectorPropertyDrawerRegistration, "valueType" | "drawer"> = {},
      ) => registerPropertyDrawer({ ...options, valueType, drawer }),
      registerField: (
        fieldType: string | string[],
        drawer: Component,
        options: Omit<InspectorPropertyDrawerRegistration, "fieldType" | "drawer"> = {},
      ) => registerPropertyDrawer({ ...options, fieldType, drawer }),
      registerAttribute: (
        attribute: string | string[],
        drawer: Component,
        options: Omit<InspectorPropertyDrawerRegistration, "attribute" | "drawer"> = {},
      ) => registerPropertyDrawer({ ...options, attribute, drawer }),
      registerPropertyPath: (
        propertyPath: string | string[],
        drawer: Component,
        options: Omit<InspectorPropertyDrawerRegistration, "propertyPath" | "drawer"> = {},
      ) => registerPropertyDrawer({ ...options, propertyPath, drawer }),
      define: defineInspectorPropertyDrawers,
    },
    unityObjectDrawer: {
      register: registerObjectDrawer,
      registerExtension: (
        extension: string | string[],
        drawer: Component,
        options: Omit<UnityObjectDrawerRegistration, "extension" | "drawer"> = {},
      ) => registerObjectDrawer(extension, drawer, options),
      define: defineUnityObjectDrawers,
    },
  };
}
