import type { Component } from "vue";
import type { UnityAssetIconKind } from "../components/icons/unityAssetIcons";
import type {
  UnityObjectPreviewLevel,
  UnityObjectPreviewModel,
  UnityObjectRefKind,
} from "../components/unity-preview/unityObjectPreview";

export interface UnityObjectDrawerContext {
  level: UnityObjectPreviewLevel;
  selected: boolean;
  disabled: boolean;
  readonly: boolean;
  draggable: boolean;
  loading: boolean;
  error: string;
}

export type UnityObjectDrawerMatcher = (
  model: UnityObjectPreviewModel,
  context: UnityObjectDrawerContext,
) => boolean;

export interface UnityObjectDrawerRegistration {
  refKind?: UnityObjectRefKind | UnityObjectRefKind[] | string | string[];
  assetKind?: UnityAssetIconKind | UnityAssetIconKind[] | string | string[];
  extension?: string | string[];
  path?: string | string[];
  title?: string | string[];
  typeLabel?: string | string[];
  level?: UnityObjectPreviewLevel | UnityObjectPreviewLevel[] | string | string[];
  drawer: Component;
  match?: UnityObjectDrawerMatcher;
  priority?: number;
}

export interface UnityObjectDrawerLibrary {
  readonly registrations: readonly UnityObjectDrawerRegistration[];
  register(registration: UnityObjectDrawerRegistration): () => void;
  register(
    extension: string | string[],
    drawer: Component,
    options?: Omit<UnityObjectDrawerRegistration, "extension" | "drawer">,
  ): () => void;
  clear(): void;
  resolve(model: UnityObjectPreviewModel, context: UnityObjectDrawerContext): Component | null;
}

export type UnityObjectDrawerInput =
  | UnityObjectDrawerRegistration[]
  | UnityObjectDrawerLibrary
  | null
  | undefined;

interface NormalizedUnityObjectDrawerRegistration {
  refKinds: string[];
  assetKinds: string[];
  extensions: string[];
  paths: string[];
  titles: string[];
  typeLabels: string[];
  levels: string[];
  drawer: Component;
  match?: UnityObjectDrawerMatcher;
  priority: number;
  order: number;
}

interface NormalizedUnityObjectDrawerRegistry {
  entries: NormalizedUnityObjectDrawerRegistration[];
  libraries: UnityObjectDrawerLibrary[];
}

const EMPTY_UNITY_OBJECT_DRAWER_REGISTRY: NormalizedUnityObjectDrawerRegistry = {
  entries: [],
  libraries: [],
};

export function defineUnityObjectDrawers(
  input: UnityObjectDrawerInput,
): UnityObjectDrawerRegistration[] {
  return expandUnityObjectDrawerRegistrations(input).map((entry) => ({
    ...entry,
    refKind: normalizeKeys(entry.refKind),
    assetKind: normalizeKeys(entry.assetKind),
    extension: normalizeExtensions(entry.extension),
    path: normalizePathKeys(entry.path),
    title: normalizeKeys(entry.title),
    typeLabel: normalizeKeys(entry.typeLabel),
    level: normalizeKeys(entry.level),
  }));
}

export function createUnityObjectDrawerLibrary(
  input?: UnityObjectDrawerInput,
): UnityObjectDrawerLibrary {
  const library = new MutableUnityObjectDrawerLibrary();
  for (const registration of expandUnityObjectDrawerRegistrations(input)) {
    library.register(registration);
  }
  return library;
}

class MutableUnityObjectDrawerLibrary implements UnityObjectDrawerLibrary {
  private readonly registeredDrawers: UnityObjectDrawerRegistration[] = [];

  get registrations(): readonly UnityObjectDrawerRegistration[] {
    return this.registeredDrawers;
  }

  register(registration: UnityObjectDrawerRegistration): () => void;
  register(
    extension: string | string[],
    drawer: Component,
    options?: Omit<UnityObjectDrawerRegistration, "extension" | "drawer">,
  ): () => void;
  register(
    registrationOrExtension: UnityObjectDrawerRegistration | string | string[],
    drawer?: Component,
    options: Omit<UnityObjectDrawerRegistration, "extension" | "drawer"> = {},
  ): () => void {
    if (typeof registrationOrExtension === "string" || Array.isArray(registrationOrExtension)) {
      if (!drawer) return () => undefined;
      return this.register({
        ...options,
        extension: registrationOrExtension,
        drawer,
      });
    }
    const registration = registrationOrExtension;
    if (!registration.drawer) return () => undefined;
    this.registeredDrawers.push(registration);
    return () => {
      const index = this.registeredDrawers.indexOf(registration);
      if (index >= 0) this.registeredDrawers.splice(index, 1);
    };
  }

  clear() {
    this.registeredDrawers.splice(0, this.registeredDrawers.length);
  }

  resolve(model: UnityObjectPreviewModel, context: UnityObjectDrawerContext): Component | null {
    return findUnityObjectDrawer(
      model,
      context,
      normalizeUnityObjectDrawers(this.registeredDrawers),
    );
  }
}

export const publicUnityObjectDrawerLibrary = createUnityObjectDrawerLibrary();
export const projectUnityObjectDrawerLibrary = publicUnityObjectDrawerLibrary;

export function normalizeUnityObjectDrawers(
  input: UnityObjectDrawerInput,
): NormalizedUnityObjectDrawerRegistry {
  if (!input) return EMPTY_UNITY_OBJECT_DRAWER_REGISTRY;
  if (isUnityObjectDrawerLibrary(input)) {
    return {
      entries: [],
      libraries: [input],
    };
  }

  const entries: NormalizedUnityObjectDrawerRegistration[] = [];
  let order = 0;
  for (const registration of expandUnityObjectDrawerRegistrations(input)) {
    if (!registration.drawer) continue;
    const normalized: NormalizedUnityObjectDrawerRegistration = {
      refKinds: normalizeKeys(registration.refKind),
      assetKinds: normalizeKeys(registration.assetKind),
      extensions: normalizeExtensions(registration.extension),
      paths: normalizePathKeys(registration.path),
      titles: normalizeKeys(registration.title),
      typeLabels: normalizeKeys(registration.typeLabel),
      levels: normalizeKeys(registration.level),
      drawer: registration.drawer,
      match: registration.match,
      priority: Number.isFinite(registration.priority) ? Number(registration.priority) : 0,
      order,
    };
    if (
      !normalized.refKinds.length &&
      !normalized.assetKinds.length &&
      !normalized.extensions.length &&
      !normalized.paths.length &&
      !normalized.titles.length &&
      !normalized.typeLabels.length &&
      !normalized.levels.length &&
      !normalized.match
    ) continue;
    entries.push(normalized);
    order += 1;
  }

  entries.sort((left, right) => right.priority - left.priority || left.order - right.order);
  return entries.length ? { entries, libraries: [] } : EMPTY_UNITY_OBJECT_DRAWER_REGISTRY;
}

export function resolveUnityObjectDrawer(
  model: UnityObjectPreviewModel,
  context: UnityObjectDrawerContext,
  input?: UnityObjectDrawerInput,
): Component | null {
  return findUnityObjectDrawer(model, context, normalizeUnityObjectDrawers(input))
    ?? publicUnityObjectDrawerLibrary.resolve(model, context);
}

export function registerUnityObjectDrawer(
  extension: string | string[],
  drawer: Component,
  options: Omit<UnityObjectDrawerRegistration, "extension" | "drawer"> = {},
): () => void {
  return publicUnityObjectDrawerLibrary.register(extension, drawer, options);
}

export const unityObjectDrawerService = {
  createLibrary: createUnityObjectDrawerLibrary,
  defineDrawers: defineUnityObjectDrawers,
  normalizeDrawers: normalizeUnityObjectDrawers,
  register: registerUnityObjectDrawer,
  publicLibrary: publicUnityObjectDrawerLibrary,
  projectLibrary: projectUnityObjectDrawerLibrary,
  resolve: resolveUnityObjectDrawer,
};

function findUnityObjectDrawer(
  model: UnityObjectPreviewModel,
  context: UnityObjectDrawerContext,
  registry: NormalizedUnityObjectDrawerRegistry,
): Component | null {
  if (!registry.entries.length && !registry.libraries.length) return null;

  const refKind = normalizeKey(model.ref.kind);
  const assetKind = normalizeKey(model.iconKind);
  const extension = normalizeExtension(fileExtension(model.ref.path || model.title));
  const path = normalizePathKey(model.ref.path);
  const title = normalizeKey(model.title);
  const typeLabel = normalizeKey(model.ref.typeLabel || model.subtitle || "");
  const level = normalizeKey(context.level);

  for (const entry of registry.entries) {
    if (
      unityObjectDrawerMatches(entry, {
        model,
        context,
        refKind,
        assetKind,
        extension,
        path,
        title,
        typeLabel,
        level,
      })
    ) return entry.drawer;
  }

  for (const library of registry.libraries) {
    const resolved = library.resolve(model, context);
    if (resolved) return resolved;
  }
  return null;
}

function unityObjectDrawerMatches(
  entry: NormalizedUnityObjectDrawerRegistration,
  target: {
    model: UnityObjectPreviewModel;
    context: UnityObjectDrawerContext;
    refKind: string;
    assetKind: string;
    extension: string;
    path: string;
    title: string;
    typeLabel: string;
    level: string;
  },
): boolean {
  if (entry.match && !entry.match(target.model, target.context)) return false;
  if (entry.refKinds.length && !entry.refKinds.some((item) => item === "*" || item === target.refKind)) return false;
  if (entry.assetKinds.length && !entry.assetKinds.some((item) => item === "*" || item === target.assetKind)) {
    return false;
  }
  if (entry.extensions.length && !entry.extensions.some((item) => item === "*" || item === target.extension)) {
    return false;
  }
  if (entry.paths.length && !entry.paths.some((item) => item === "*" || item === target.path)) return false;
  if (entry.titles.length && !entry.titles.some((item) => item === "*" || item === target.title)) return false;
  if (entry.typeLabels.length && !entry.typeLabels.some((item) => item === "*" || item === target.typeLabel)) {
    return false;
  }
  if (entry.levels.length && !entry.levels.some((item) => item === "*" || item === target.level)) return false;
  return true;
}

function expandUnityObjectDrawerRegistrations(input: UnityObjectDrawerInput): UnityObjectDrawerRegistration[] {
  if (!input) return [];
  if (isUnityObjectDrawerLibrary(input)) return [...input.registrations];
  return Array.isArray(input) ? [...input] : [];
}

function isUnityObjectDrawerLibrary(value: unknown): value is UnityObjectDrawerLibrary {
  return Boolean(
    value &&
    typeof value === "object" &&
    "registrations" in value &&
    "register" in value &&
    "resolve" in value,
  );
}

function normalizeKeys(value: string | string[] | undefined): string[] {
  return (Array.isArray(value) ? value : [value])
    .map((item) => normalizeKey(item || ""))
    .filter(Boolean);
}

function normalizePathKeys(value: string | string[] | undefined): string[] {
  return (Array.isArray(value) ? value : [value])
    .map((item) => normalizePathKey(item || ""))
    .filter(Boolean);
}

function normalizeExtensions(value: string | string[] | undefined): string[] {
  return (Array.isArray(value) ? value : [value])
    .map((item) => normalizeExtension(item || ""))
    .filter(Boolean);
}

function normalizeKey(value: string): string {
  return value.trim().toLowerCase();
}

function normalizePathKey(value: string): string {
  return value.trim().replace(/\\/g, "/").replace(/\/+$/g, "").toLowerCase();
}

function normalizeExtension(value: string): string {
  const normalized = normalizeKey(value);
  if (!normalized) return "";
  return normalized.startsWith(".") ? normalized : `.${normalized}`;
}

function fileExtension(path: string): string {
  const leaf = path.trim().replace(/\\/g, "/").split("/").filter(Boolean).pop() ?? "";
  const dot = leaf.lastIndexOf(".");
  return dot >= 0 ? leaf.slice(dot) : "";
}
