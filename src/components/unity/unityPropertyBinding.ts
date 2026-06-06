import { h, type VNodeChild } from "vue";
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
  type UnityPropertyPathTargetKind,
} from "../../services/unityPropertyPath";
import UnityPropertyEditor from "./UnityPropertyEditor.vue";
import UnitySerializedPropertyTree from "./UnitySerializedPropertyTree.vue";
import type { UnitySerializedPropertyCommitEvent } from "./unitySerializedValue";

export type UnityPropertyWriteMode = "commit" | "preview";
export type UnityPropertyPathInput = string | UnitySerializedPropertyTarget;
export type { UnityPropertyPathTargetKind };

export interface UnityBoundPropertyReadRequest {
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
}

export interface UnityBoundPropertyApplyOptions {
  label?: string;
  undoable?: boolean;
  refresh?: boolean;
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
  const parts = [
    target.kind,
    target.guid ?? "",
    target.path ?? "",
    target.scenePath ?? "",
    target.objectPath ?? "",
    target.componentType ?? "",
    String(target.componentIndex ?? 0),
  ]
    .filter(Boolean)
    .join(":")
    .replace(/\s+/g, " ")
    .trim();
  return `${prefix}:${parts || "target"}`;
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
  readonly raw: InspectorProperty;

  constructor(tree: UnityBoundPropertyTree, raw: InspectorProperty) {
    this.tree = tree;
    this.raw = raw;
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
          void this.preview(commit.value);
          return;
        }
        options.onCommit?.(event);
        void this.tree.writeCommit(commit, writeOptionsFromDrawOptions(options));
      },
    });
  }
}

export class UnityBoundPropertyTree {
  readonly adapter: UnityBoundPropertyRuntimeAdapter;
  readonly bindingId: string;
  readonly target: UnitySerializedPropertyTarget;
  readonly options: UnityBoundPropertyRuntimeOptions;
  snapshots: InspectorPropertyTreeSnapshotInput | null;
  raw: PropertyTree;

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
    this.snapshots = snapshots;
    this.raw = this.createTree(snapshots);
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
    this.snapshots = snapshotsFromReadResult(result);
    this.raw = this.createTree(this.snapshots);
    return this;
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
    const result = await this.adapter.write({
      bindingId: this.bindingId,
      target,
      value: commit.value,
      writeMode: mode,
    }, {
      ...options,
      beforeSnapshot: options.beforeSnapshot ?? commit.snapshot,
    });
    if (mode === "commit" && options.refresh !== false) {
      await this.refresh();
    }
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
        void this.writeCommit(commit, writeOptionsFromDrawOptions(options));
      },
      onPreview: (value: unknown) => {
        const commit = property.createCommit(value);
        const target = this.targetForProperty(property);
        const event = eventFromCommit(commit, target);
        event.writeMode = "preview";
        options.onPreview?.(event);
        void this.writeCommit(commit, { refresh: false, undoable: false }, "preview");
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
      commit: async (commit) => {
        await this.writeCommit(commit, { refresh: true });
      },
    });
  }

  private handleDrawEvent(
    event: UnitySerializedPropertyCommitEvent,
    options: UnityBoundPropertyDrawOptions,
    mode: UnityPropertyWriteMode,
  ) {
    const property = this.raw.getProperty(event.propertyPath)
      ?? this.raw.getProperty(event.property.propertyPath)
      ?? this.raw.rootProperty;
    if (!property) return;
    const commit = property.createCommit(event.value);
    if (mode === "preview") {
      options.onPreview?.({ ...event, writeMode: "preview" });
      void this.writeCommit(commit, { refresh: false, undoable: false }, "preview");
      return;
    }
    options.onCommit?.({ ...event, writeMode: "commit" });
    void this.writeCommit(commit, writeOptionsFromDrawOptions(options));
  }
}

export interface UnityPropertyRuntime {
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
    return new UnityBoundPropertyTree(
      adapter,
      target,
      snapshotsFromReadResult(result),
      options,
    );
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
    return tree.get(target.propertyPath) ?? tree.root ?? (() => {
      throw new Error(`Unity property not found: ${target.propertyPath}`);
    })();
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
