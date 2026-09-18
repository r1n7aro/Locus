/// <reference lib="es2021.weakref" />
import { h, markRaw, shallowReactive, type VNodeChild } from "vue";
import {
  createInspectorPropertyTreeBinding,
  createPropertyTree,
  type InspectorProperty,
  type InspectorPropertyCommit,
  type InspectorPropertyDrawerInput,
  type InspectorPropertySnapshot,
  type InspectorPropertyTreeBindingInput,
  type InspectorPropertyTreeOptions,
  type InspectorPropertyTreeSnapshotInput,
  type PropertyTree,
} from "../../services/propertyTree";
import type { UnitySerializedPropertyTarget } from "../../services/unitySerializedProperty";
import {
  resolveUnityPropertyTarget,
  unityPropertyObjectTarget,
  unityPropertyTargetWithPath,
  unityPropertyTargetKey,
  type UnityPropertyPathTargetKind,
} from "../../services/unityPropertyPath";
import UnityPropertyEditor from "./UnityPropertyEditor.vue";
import UnitySerializedPropertyTree from "./UnitySerializedPropertyTree.vue";
import type { UnitySerializedPropertyCommitEvent } from "./unitySerializedValue";

export type UnityPropertyWriteMode = "commit" | "preview";
export type UnityPropertyPathInput = string | UnitySerializedPropertyTarget;
export type { UnityPropertyPathTargetKind };

export interface UnityBoundPropertyReadRequest {
  arrayOffset?: number | null;
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  maxDepth?: number | null;
  maxArrayItems?: number | null;
}

export interface UnityBoundPropertyWriteRequest {
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  value: unknown;
  writeMode?: UnityPropertyWriteMode | null;
}

export interface UnityBoundPropertyApplyWrite {
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  value: unknown;
  writeMode?: UnityPropertyWriteMode | null;
}

export interface UnityBoundPropertyApplyRequest {
  writes: UnityBoundPropertyApplyWrite[];
}

export interface UnityBoundPropertyReadResult extends InspectorPropertySnapshot {
  ok?: boolean;
  bindingId?: string | null;
  message?: string;
  target?: UnitySerializedPropertyTarget | null;
  properties?: InspectorPropertySnapshot[];
}

export interface UnityBoundPropertyWriteResult extends UnityBoundPropertyReadResult {
  saved?: boolean;
}

export interface UnityBoundPropertyApplyResult {
  ok: boolean;
  message?: string;
  results: UnityBoundPropertyWriteResult[];
}

export interface UnityBoundPropertyWriteOptions {
  label?: string;
  undoable?: boolean;
  beforeSnapshot?: InspectorPropertySnapshot | null;
  refresh?: boolean;
  onApplied?: (result: UnityBoundPropertyWriteResult) => void | Promise<void>;
}

export interface UnityBoundPropertyApplyOptions {
  label?: string;
  undoable?: boolean;
  refresh?: boolean;
  onApplied?: (result: UnityBoundPropertyApplyResult) => void | Promise<void>;
}

export interface UnityBoundPropertyRuntimeAdapter {
  read(request: UnityBoundPropertyReadRequest): Promise<UnityBoundPropertyReadResult>;
  write(
    request: UnityBoundPropertyWriteRequest,
    options?: UnityBoundPropertyWriteOptions,
  ): Promise<UnityBoundPropertyWriteResult>;
  apply(
    request: UnityBoundPropertyApplyRequest,
    options?: UnityBoundPropertyApplyOptions,
  ): Promise<UnityBoundPropertyApplyResult>;
  undo?: () => unknown | Promise<unknown>;
  redo?: () => unknown | Promise<unknown>;
}

export interface UnityBoundPropertyRuntimeOptions {
  idPrefix?: string;
  maxDepth?: number;
  maxArrayItems?: number;
  treeOptions?: Omit<InspectorPropertyTreeOptions, "id" | "targetId">;
}

export interface UnityBoundPropertyDrawOptions {
  propertyDrawers?: InspectorPropertyDrawerInput;
  disabled?: boolean;
  readonly?: boolean;
  compact?: boolean;
  showLabel?: boolean;
  writeMode?: UnityPropertyWriteMode;
  onCommit?: (event: UnitySerializedPropertyCommitEvent) => void;
  onPreview?: (event: UnitySerializedPropertyCommitEvent) => void;
}

function normalizeBindingId(target: UnitySerializedPropertyTarget, prefix = "unity-property"): string {
  // Tag every field (including file ids) so distinct targets can never share a binding id,
  // e.g. {path: "X"} vs {scenePath: "X"} or two sub-objects differing only by fileId.
  const parts = [
    `id=${target.globalObjectId ?? ""}`,
    `k=${target.kind}`,
    `g=${target.guid ?? ""}`,
    `p=${target.path ?? ""}`,
    `s=${target.scenePath ?? ""}`,
    `o=${target.objectPath ?? ""}`,
    `of=${target.objectFileId ?? 0}`,
    `tf=${target.targetFileId ?? 0}`,
    `c=${target.componentType ?? ""}`,
    `ci=${target.componentIndex ?? 0}`,
  ]
    .join(":")
    .replace(/\s+/g, " ")
    .trim();
  return `${prefix}:${parts}`;
}

function warnUnityPropertyWriteFailure(error: unknown) {
  console.warn("[unityPropertyBinding] serialized property write failed:", error);
}

function snapshotList(
  input: InspectorPropertyTreeSnapshotInput | null | undefined,
): InspectorPropertySnapshot[] {
  if (!input) return [];
  return Array.isArray(input) ? input : [input];
}

function snapshotsFromReadResult(result: UnityBoundPropertyReadResult): InspectorPropertyTreeSnapshotInput {
  return Array.isArray(result.properties) && result.properties.length
    ? result.properties
    : result;
}

function snapshotTarget(snapshot: InspectorPropertySnapshot): UnitySerializedPropertyTarget | null {
  return (snapshot.bindingTarget ?? snapshot.target ?? null) as UnitySerializedPropertyTarget | null;
}

function eventFromCommit(
  commit: InspectorPropertyCommit,
  target: UnitySerializedPropertyTarget,
): UnitySerializedPropertyCommitEvent {
  return {
    propertyPath: commit.propertyPath,
    value: commit.value,
    property: commit.snapshot as UnitySerializedPropertyCommitEvent["property"],
    target,
    writeMode: "commit",
  };
}

function propertyType(property: InspectorProperty): string {
  return property.valueType || property.type || "String";
}

function writeOptionsFromDrawOptions(_options: UnityBoundPropertyDrawOptions): UnityBoundPropertyWriteOptions {
  return {};
}

export class UnityBoundProperty {
  readonly tree: UnityBoundPropertyTree;
  private readonly initialProperty: InspectorProperty;

  get raw(): InspectorProperty {
    return this.tree.resolveProperty(this.initialProperty) ?? this.initialProperty;
  }

  constructor(tree: UnityBoundPropertyTree, raw: InspectorProperty) {
    this.tree = tree;
    this.initialProperty = raw;
  }

  get propertyPath(): string {
    return this.raw.propertyPath;
  }

  get value(): unknown {
    return this.raw.value;
  }

  get target(): UnitySerializedPropertyTarget {
    return this.tree.targetForProperty(this.raw);
  }

  createCommit(value: unknown): InspectorPropertyCommit {
    return this.raw.createCommit(value);
  }

  async write(value: unknown, options: UnityBoundPropertyWriteOptions = {}) {
    const result = await this.tree.writeProperty(this.raw, value, {
      ...options,
      refresh: options.refresh ?? true,
    });
    return result;
  }

  async preview(value: unknown, options: Omit<UnityBoundPropertyWriteOptions, "refresh"> = {}) {
    return this.tree.writeProperty(this.raw, value, {
      ...options,
      refresh: false,
      undoable: false,
    }, "preview");
  }

  async undo() {
    await this.tree.undo();
  }

  async redo() {
    await this.tree.redo();
  }

  drawDefaultEditor(options: UnityBoundPropertyDrawOptions = {}): VNodeChild {
    return this.tree.drawPropertyEditor(this.raw, options);
  }

  draw(options: UnityBoundPropertyDrawOptions = {}): VNodeChild {
    return this.raw.draw({
      drawers: options.propertyDrawers,
      disabled: options.disabled,
      readonly: options.readonly,
      compact: options.compact,
      showLabel: options.showLabel,
      onCommit: (commit) => {
        const target = this.tree.targetForProperty(commit.property);
        const event = eventFromCommit(commit, target);
        if (options.writeMode === "preview") {
          event.writeMode = "preview";
          options.onPreview?.(event);
          void this.preview(commit.value).catch(warnUnityPropertyWriteFailure);
          return;
        }
        options.onCommit?.(event);
        void this.tree.writeCommit(commit, writeOptionsFromDrawOptions(options))
          .catch(warnUnityPropertyWriteFailure);
      },
    });
  }
}

export class UnityBoundPropertyTree {
  readonly adapter: UnityBoundPropertyRuntimeAdapter;
  readonly bindingId: string;
  readonly target: UnitySerializedPropertyTarget;
  readonly options: UnityBoundPropertyRuntimeOptions;
  private readonly state: { snapshots: InspectorPropertyTreeSnapshotInput | null; raw: PropertyTree };
  get snapshots() { return this.state.snapshots; }
  get raw() { return this.state.raw; }

  constructor(
    adapter: UnityBoundPropertyRuntimeAdapter,
    target: UnitySerializedPropertyTarget,
    snapshots: InspectorPropertyTreeSnapshotInput | null,
    options: UnityBoundPropertyRuntimeOptions = {},
  ) {
    this.adapter = adapter;
    this.target = unityPropertyObjectTarget(target);
    this.bindingId = normalizeBindingId(this.target, options.idPrefix);
    this.options = options;
    this.state = shallowReactive({ snapshots, raw: this.createTree(snapshots) });
    markRaw(this);
  }

  resolveProperty(property: InspectorProperty): InspectorProperty | null {
    const target = unityPropertyTargetKey(this.targetForProperty(property));
    return this.raw.properties.find((candidate) => candidate.propertyPath === property.propertyPath
      && unityPropertyTargetKey(this.targetForProperty(candidate)) === target) ?? null;
  }

  get root(): UnityBoundProperty | null {
    return this.raw.rootProperty ? new UnityBoundProperty(this, this.raw.rootProperty) : null;
  }

  get properties(): UnityBoundProperty[] {
    return this.raw.properties.map((property) => new UnityBoundProperty(this, property));
  }

  require(propertyPath: string): UnityBoundProperty {
    return new UnityBoundProperty(this, this.raw.requireProperty(propertyPath));
  }

  get(propertyPath: string): UnityBoundProperty | null {
    const property = this.raw.getProperty(propertyPath);
    return property ? new UnityBoundProperty(this, property) : null;
  }

  targetForProperty(property: InspectorProperty): UnitySerializedPropertyTarget {
    const target = snapshotTarget(property.snapshot)
      ?? snapshotTarget(property.root.snapshot)
      ?? this.target;
    return unityPropertyTargetWithPath(target, property.propertyPath);
  }

  async refresh() {
    const result = await this.adapter.read({
      bindingId: this.bindingId,
      target: this.target,
      maxDepth: this.options.maxDepth,
      maxArrayItems: this.options.maxArrayItems,
    });
    if (result.ok === false) throw new Error(result.message || "Failed to read Unity properties.");
    this.state.snapshots = snapshotsFromReadResult(result);
    this.state.raw = this.createTree(this.snapshots);
    return this;
  }

  async loadChildren(property: InspectorProperty) {
    const target = this.targetForProperty(property);
    const arrayOffset = property.isArray ? property.children.length : 0;
    const result = await this.adapter.read({
      bindingId: this.bindingId, target,
      maxDepth: Math.min(16, Math.max(4, (this.options.maxDepth ?? 4) * 2)),
      maxArrayItems: Math.min(1024, Math.max(128, property.children.length * 2)),
      arrayOffset,
    });
    if (result.ok === false) throw new Error(result.message || "Failed to read Unity properties.");
    if (property.isArray && result.arraySize !== property.arraySize) { await this.refresh(); return; }
    const loaded = arrayOffset ? { ...result, children: [...property.children.map((child) => child.snapshot), ...(result.children ?? [])], visibleChildCount: property.children.length + (result.children?.length ?? 0) } : result;
    const key = unityPropertyTargetKey(target);
    const replace = (snapshot: InspectorPropertySnapshot, owner = this.target): InspectorPropertySnapshot => {
      const current = snapshotTarget(snapshot) ?? owner;
      if (unityPropertyTargetKey(unityPropertyTargetWithPath(current, snapshot.propertyPath)) === key) return loaded;
      return { ...snapshot, children: snapshot.children?.map((child) => replace(child, current)) };
    };
    this.state.snapshots = Array.isArray(this.snapshots) ? this.snapshots.map((root) => replace(root)) : this.snapshots ? replace(this.snapshots) : null;
    this.state.raw = this.createTree(this.snapshots);
  }

  async writeProperty(
    property: InspectorProperty,
    value: unknown,
    options: UnityBoundPropertyWriteOptions = {},
    mode: UnityPropertyWriteMode = "commit",
  ) {
    const commit = property.createCommit(value);
    return this.writeCommit(commit, options, mode);
  }

  async writeCommit(
    commit: InspectorPropertyCommit,
    options: UnityBoundPropertyWriteOptions = {},
    mode: UnityPropertyWriteMode = "commit",
  ) {
    const target = this.targetForProperty(commit.property);
    let notified = false;
    const onApplied = async (applied: UnityBoundPropertyWriteResult) => {
      notified = true;
      if (mode === "commit" && options.refresh !== false) await this.refresh();
      await options.onApplied?.(applied);
    };
    const result = await this.adapter.write({
      bindingId: this.bindingId,
      target,
      value: commit.value,
      writeMode: mode,
    }, {
      ...options,
      beforeSnapshot: options.beforeSnapshot ?? commit.snapshot,
      onApplied,
    });
    if (!notified) await onApplied(result);
    return result;
  }

  async apply(
    writes: UnityBoundPropertyApplyWrite[],
    options: UnityBoundPropertyApplyOptions = {},
  ) {
    const result = await this.adapter.apply({ writes }, options);
    if (options.refresh !== false) {
      await this.refresh();
    }
    return result;
  }

  async undo() {
    await this.adapter.undo?.();
  }

  async redo() {
    await this.adapter.redo?.();
  }

  drawDefaultEditor(options: UnityBoundPropertyDrawOptions = {}): VNodeChild {
    if (this.raw.rootProperties.length > 1) {
      return h("div", { class: "unity-property-tree-roots" }, this.raw.rootProperties.map((property) =>
        h(UnitySerializedPropertyTree, {
          source: this.bindingInput(property), propertyDrawers: options.propertyDrawers,
          disabled: options.disabled, readonly: options.readonly, compact: options.compact,
          onCommit: (event: UnitySerializedPropertyCommitEvent) => this.handleDrawEvent(event, options, "commit"),
          onPreview: (event: UnitySerializedPropertyCommitEvent) => this.handleDrawEvent(event, options, "preview"),
        })));
    }
    return h(UnitySerializedPropertyTree, {
      source: this.bindingInput(),
      propertyDrawers: options.propertyDrawers,
      disabled: options.disabled,
      readonly: options.readonly,
      compact: options.compact,
      onCommit: (event: UnitySerializedPropertyCommitEvent) => this.handleDrawEvent(event, options, "commit"),
      onPreview: (event: UnitySerializedPropertyCommitEvent) => this.handleDrawEvent(event, options, "preview"),
    });
  }

  drawPropertyEditor(property: InspectorProperty, options: UnityBoundPropertyDrawOptions = {}): VNodeChild {
    if (property.children.length || property.isArray || property.isManagedReference || property.drawer.container) {
      return h(UnitySerializedPropertyTree, {
        source: this.bindingInput(property),
        propertyDrawers: options.propertyDrawers,
        disabled: options.disabled,
        readonly: options.readonly,
        compact: options.compact,
        hideRootObjectHeader: options.showLabel === false,
        onCommit: (event: UnitySerializedPropertyCommitEvent) => this.handleDrawEvent(event, options, "commit"),
        onPreview: (event: UnitySerializedPropertyCommitEvent) => this.handleDrawEvent(event, options, "preview"),
      });
    }

    return h(UnityPropertyEditor, {
      modelValue: property.value,
      propertyType: propertyType(property),
      displayValue: property.displayValue,
      bindingTarget: this.targetForProperty(property),
      editable: property.editable,
      disabled: options.disabled,
      readonly: options.readonly,
      enumOptions: property.enumOptions,
      isFlagsEnum: property.isFlagsEnum,
      enumValueIndex: property.enumValueIndex,
      enumValueFlag: property.enumValueFlag,
      title: property.propertyPath,
      tooltip: property.tooltip,
      hasRange: property.hasRange,
      rangeMin: property.rangeMin,
      rangeMax: property.rangeMax,
      numberStep: property.numberStep,
      multiline: property.multiline,
      minLines: property.minLines,
      maxLines: property.maxLines,
      referenceTypeFullName: property.referenceTypeFullName,
      referenceTypeAssembly: property.referenceTypeAssembly,
      onCommit: (value: unknown) => {
        const commit = property.createCommit(value);
        const target = this.targetForProperty(property);
        options.onCommit?.(eventFromCommit(commit, target));
        void this.writeCommit(commit, writeOptionsFromDrawOptions(options))
          .catch(warnUnityPropertyWriteFailure);
      },
      onPreview: (value: unknown) => {
        const commit = property.createCommit(value);
        const target = this.targetForProperty(property);
        const event = eventFromCommit(commit, target);
        event.writeMode = "preview";
        options.onPreview?.(event);
        void this.writeCommit(commit, { refresh: false, undoable: false }, "preview")
          .catch(warnUnityPropertyWriteFailure);
      },
    });
  }

  private createTree(snapshots: InspectorPropertyTreeSnapshotInput | null | undefined): PropertyTree {
    return createPropertyTree(snapshots, {
      id: this.bindingId,
      targetId: this.bindingId,
      ...(this.options.treeOptions ?? {}),
    });
  }

  private bindingInput(property?: InspectorProperty): InspectorPropertyTreeBindingInput {
    return createInspectorPropertyTreeBinding({
      id: property ? `${this.bindingId}:${property.propertyPath}` : this.bindingId,
      targetId: this.bindingId,
      snapshots: property?.snapshot ?? this.snapshots,
      disabled: this.options.treeOptions?.disabled,
      readonly: this.options.treeOptions?.readonly,
      editable: this.options.treeOptions?.readonly === true ? false : undefined,
      loadChildren: (node) => this.loadChildren(node),
    });
  }

  private handleDrawEvent(
    event: UnitySerializedPropertyCommitEvent,
    options: UnityBoundPropertyDrawOptions,
    mode: UnityPropertyWriteMode,
  ) {
    // Never fall back to the root property: a stale event after refresh would
    // otherwise commit a child value onto the root target's propertyPath.
    const candidates = this.raw.properties.filter((candidate) => candidate.propertyPath === event.propertyPath);
    const eventTarget = event.target ?? snapshotTarget(event.property);
    const property = eventTarget ? candidates.find((candidate) =>
      unityPropertyTargetKey(this.targetForProperty(candidate)) === unityPropertyTargetKey(unityPropertyTargetWithPath(eventTarget, candidate.propertyPath)))
      : candidates.length === 1 ? candidates[0] : null;
    if (!property) {
      console.warn(
        `[unityPropertyBinding] dropped ${mode} for unknown property path: ${event.propertyPath}`,
      );
      return;
    }
    const commit = property.createCommit(event.value);
    if (mode === "preview") {
      options.onPreview?.({ ...event, writeMode: "preview" });
      void this.writeCommit(commit, { refresh: false, undoable: false }, "preview")
        .catch(warnUnityPropertyWriteFailure);
      return;
    }
    options.onCommit?.({ ...event, writeMode: "commit" });
    void this.writeCommit(commit, writeOptionsFromDrawOptions(options))
      .catch(warnUnityPropertyWriteFailure);
  }
}

export interface UnityPropertyRuntime {
  refreshTargets(target: UnitySerializedPropertyTarget): Promise<void>;
  parsePath(path: string): UnitySerializedPropertyTarget;
  objectTarget(input: UnityPropertyPathInput): UnitySerializedPropertyTarget;
  write(
    input: UnityPropertyPathInput,
    value: unknown,
    options?: UnityBoundPropertyWriteOptions & { writeMode?: UnityPropertyWriteMode },
  ): Promise<UnityBoundPropertyWriteResult>;
  apply(
    writes: UnityBoundPropertyApplyWrite[],
    options?: UnityBoundPropertyApplyOptions,
  ): Promise<UnityBoundPropertyApplyResult>;
  readTree(
    input: UnityPropertyPathInput,
    options?: UnityBoundPropertyRuntimeOptions,
  ): Promise<UnityBoundPropertyTree>;
  fromPath(
    input: UnityPropertyPathInput,
    options?: UnityBoundPropertyRuntimeOptions,
  ): Promise<UnityBoundPropertyTree>;
  readProperty(
    input: UnityPropertyPathInput,
    options?: UnityBoundPropertyRuntimeOptions,
  ): Promise<UnityBoundProperty>;
  property(
    input: UnityPropertyPathInput,
    options?: UnityBoundPropertyRuntimeOptions,
  ): Promise<UnityBoundProperty>;
}

export function createUnityPropertyRuntime(
  adapter: UnityBoundPropertyRuntimeAdapter,
): UnityPropertyRuntime {
  const trees = new Set<WeakRef<UnityBoundPropertyTree>>();
  async function refreshTargets(target: UnitySerializedPropertyTarget) {
    const jobs: Promise<unknown>[] = [];
    for (const reference of trees) {
      const tree = reference.deref();
      if (!tree) { trees.delete(reference); continue; }
      if (tree.raw.properties.some((property) => {
        const own = tree.targetForProperty(property);
        return own.globalObjectId && target.globalObjectId ? own.globalObjectId === target.globalObjectId
          : unityPropertyTargetKey(unityPropertyObjectTarget(own)) === unityPropertyTargetKey(unityPropertyObjectTarget(target));
      })) jobs.push(tree.refresh());
    }
    await Promise.all(jobs);
  }
  async function readTree(
    input: UnityPropertyPathInput,
    options: UnityBoundPropertyRuntimeOptions = {},
  ): Promise<UnityBoundPropertyTree> {
    const target = resolveUnityPropertyTarget(input);
    const result = await adapter.read({
      bindingId: normalizeBindingId(unityPropertyObjectTarget(target), options.idPrefix),
      target,
      maxDepth: options.maxDepth,
      maxArrayItems: options.maxArrayItems,
    });
    if (result.ok === false) throw new Error(result.message || "Failed to read Unity properties.");
    const tree = new UnityBoundPropertyTree(
      adapter,
      result.target ?? target,
      snapshotsFromReadResult(result),
      options,
    );
    trees.add(new WeakRef(tree));
    return tree;
  }

  async function readProperty(
    input: UnityPropertyPathInput,
    options: UnityBoundPropertyRuntimeOptions = {},
  ): Promise<UnityBoundProperty> {
    const target = resolveUnityPropertyTarget(input);
    if (!target.propertyPath) {
      throw new Error("Unity property path target requires propertyPath.");
    }
    const tree = await readTree(target, options);
    const property = tree.get(target.propertyPath);
    if (!property) {
      throw new Error(`Unity property not found: ${target.propertyPath}`);
    }
    return property;
  }

  async function write(
    input: UnityPropertyPathInput,
    value: unknown,
    options: UnityBoundPropertyWriteOptions & { writeMode?: UnityPropertyWriteMode } = {},
  ) {
    const target = resolveUnityPropertyTarget(input);
    if (!target.propertyPath) {
      throw new Error("Unity property write target requires propertyPath.");
    }
    return adapter.write({
      bindingId: normalizeBindingId(unityPropertyObjectTarget(target)),
      target,
      value,
      writeMode: options.writeMode ?? "commit",
    }, options);
  }

  async function apply(
    writes: UnityBoundPropertyApplyWrite[],
    options: UnityBoundPropertyApplyOptions = {},
  ) {
    return adapter.apply({ writes }, options);
  }

  return {
    refreshTargets,
    parsePath: resolveUnityPropertyTarget,
    objectTarget: unityPropertyObjectTarget,
    write,
    apply,
    readTree,
    fromPath: readTree,
    readProperty,
    property: readProperty,
  };
}

export function unityBoundPropertySnapshots(
  tree: UnityBoundPropertyTree | null | undefined,
): InspectorPropertySnapshot[] {
  return snapshotList(tree?.snapshots ?? null);
}
