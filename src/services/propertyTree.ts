import { h, type Component, type VNodeChild } from "vue";

export type InspectorDrawerKind =
  | "array"
  | "boolean"
  | "color"
  | "enum"
  | "flags"
  | "layerMask"
  | "managedReference"
  | "number"
  | "object"
  | "objectReference"
  | "text"
  | "unsupported"
  | "vector";

export type InspectorCommitMode = "none" | "change" | "blur" | "command";

export interface InspectorManagedReferenceTypeOption {
  label: string;
  value: string;
  fullName?: string;
  assembly?: string;
  current?: boolean;
  unavailable?: boolean;
}

export interface InspectorPropertyAttributeInfo {
  type?: string;
  displayName?: string;
  value?: string;
}

export interface InspectorPropertyTargetSnapshot {
  kind: string;
  path?: string | null;
  scenePath?: string | null;
  objectPath?: string | null;
  objectFileId?: number | null;
  targetFileId?: number | null;
  componentType?: string | null;
  componentIndex?: number | null;
  propertyPath?: string | null;
}

export interface InspectorSelectOption {
  label: string;
  value: string;
  name?: string;
  index?: number;
  numericValue?: number;
}

export type InspectorSelectOptionInput = {
  label?: string;
  value?: string;
  name?: string;
  index?: number;
  numericValue?: number;
};

export interface InspectorPropertySnapshot {
  propertyPath: string;
  bindingTarget?: InspectorPropertyTargetSnapshot | null;
  target?: InspectorPropertyTargetSnapshot | null;
  displayName?: string;
  name?: string;
  type?: string;
  valueType?: string;
  fieldTypeFullName?: string;
  fieldTypeAssembly?: string;
  value?: unknown;
  displayValue?: string;
  editable?: boolean;
  hasChildren?: boolean;
  isArray?: boolean;
  arraySize?: number;
  isFlagsEnum?: boolean;
  enumValueIndex?: number;
  enumValueFlag?: number;
  enumOptions?: InspectorSelectOptionInput[];
  children?: InspectorPropertySnapshot[];
  isManagedReference?: boolean;
  managedReferenceFullTypename?: string;
  managedReferenceFieldTypename?: string;
  managedReferenceDisplayName?: string;
  managedReferenceTypes?: InspectorManagedReferenceTypeOption[];
  tooltip?: string;
  header?: string;
  hasRange?: boolean;
  rangeMin?: number;
  rangeMax?: number;
  numberStep?: number;
  multiline?: boolean;
  minLines?: number;
  maxLines?: number;
  referenceTypeFullName?: string;
  referenceTypeAssembly?: string;
  attributes?: InspectorPropertyAttributeInfo[];
}

export interface InspectorPropertyResolvedDrawer {
  kind: InspectorDrawerKind;
  commitMode: InspectorCommitMode;
  container: boolean;
  valueType: string;
}

export interface InspectorPropertyState {
  editable: boolean;
  disabled: boolean;
  readonly: boolean;
  expanded: boolean;
  selected: boolean;
  changed: boolean;
  matchesSearch: boolean;
  hasVisibleDescendant: boolean;
  visible: boolean;
  error: string;
  warning: string;
}

export interface InspectorPropertyCommit {
  propertyPath: string;
  value: unknown;
  property: InspectorProperty;
  snapshot: InspectorPropertySnapshot;
}

export type InspectorPropertyTreeSnapshotInput =
  | InspectorPropertySnapshot
  | InspectorPropertySnapshot[];

export type InspectorPropertyTreeCommitHandler = (
  commit: InspectorPropertyCommit,
) => void | Promise<void>;

export interface InspectorPropertyTreeBindingInput {
  id?: string;
  targetId?: string;
  snapshots?: InspectorPropertyTreeSnapshotInput | null;
  loading?: boolean;
  error?: string;
  disabled?: boolean;
  readonly?: boolean;
  editable?: boolean;
  commit?: InspectorPropertyTreeCommitHandler | null;
}

export interface InspectorPropertyTreeBinding {
  id: string;
  targetId: string;
  snapshots: InspectorPropertyTreeSnapshotInput | null;
  loading: boolean;
  error: string;
  disabled: boolean;
  readonly: boolean;
  editable: boolean;
  commit: InspectorPropertyTreeCommitHandler;
}

export type InspectorPropertyDrawerMatcher = (
  property: InspectorProperty,
  context: InspectorPropertyTreeContext,
) => boolean;

export type InspectorPropertyDrawerResolver = (
  property: InspectorProperty,
  context: InspectorPropertyTreeContext,
) => Component | null | undefined;

export interface InspectorPropertyDrawerRegistration {
  type?: string | string[];
  valueType?: string | string[];
  fieldType?: string | string[];
  attribute?: string | string[];
  propertyPath?: string | string[];
  name?: string | string[];
  drawerKind?: InspectorDrawerKind | InspectorDrawerKind[] | string | string[];
  drawer: Component;
  match?: InspectorPropertyDrawerMatcher;
  priority?: number;
}

export interface InspectorPropertyDrawerLibrary {
  readonly registrations: readonly InspectorPropertyDrawerRegistration[];
  register(registration: InspectorPropertyDrawerRegistration): () => void;
  register(
    type: string | string[],
    drawer: Component,
    options?: Omit<InspectorPropertyDrawerRegistration, "type" | "drawer">,
  ): () => void;
  clear(): void;
  resolve(property: InspectorProperty, context: InspectorPropertyTreeContext): Component | null;
}

export type InspectorPropertyDrawerInput =
  | Record<string, Component>
  | Map<string, Component>
  | InspectorPropertyDrawerRegistration[]
  | InspectorPropertyDrawerLibrary
  | null
  | undefined;

export interface InspectorPropertyDrawerOptions {
  drawer?: Component | null;
  drawers?: InspectorPropertyDrawerInput;
  disabled?: boolean;
  readonly?: boolean;
  compact?: boolean;
  showLabel?: boolean;
  onCommit?: (commit: InspectorPropertyCommit) => void;
}

export interface InspectorPropertyDrawerProps {
  property: InspectorProperty;
  snapshot: InspectorPropertySnapshot;
  propertyPath: string;
  label: string;
  value: unknown;
  modelValue: unknown;
  displayValue: string;
  type: string;
  valueType: string;
  propertyType: string;
  fieldTypeFullName: string;
  fieldTypeAssembly: string;
  tooltip: string;
  header: string;
  hasRange: boolean;
  rangeMin: number;
  rangeMax: number;
  numberStep: number;
  multiline: boolean;
  minLines: number;
  maxLines: number;
  referenceTypeFullName: string;
  referenceTypeAssembly: string;
  attributes: InspectorPropertyAttributeInfo[];
  editable: boolean;
  disabled: boolean;
  readonly: boolean;
  compact: boolean;
  treeId: string;
  targetId: string;
  depth: number;
  children: InspectorProperty[];
  commit: (value: unknown, property?: InspectorProperty) => void;
  draw: (property?: InspectorProperty, options?: InspectorPropertyDrawerOptions) => VNodeChild;
  drawChild: (propertyOrPath: InspectorProperty | string, options?: InspectorPropertyDrawerOptions) => VNodeChild;
}

export interface InspectorManagedReferenceTypeSearchOptions {
  limit?: number;
}

export interface InspectorPropertyTreeFlattenOptions {
  visibleOnly?: boolean;
  includeContainers?: boolean;
}

export interface InspectorPropertyTreeOptions {
  id?: string;
  targetId?: string;
  readonly?: boolean;
  disabled?: boolean;
  searchQuery?: string;
  selectedPath?: string | null;
  expandedPaths?: InspectorPathSetInput;
  collapsedPaths?: InspectorPathSetInput;
  changedPaths?: InspectorPathSetInput;
  errors?: InspectorMessageInput;
  warnings?: InspectorMessageInput;
  defaultExpandedDepth?: number;
  autoCollapseChildCount?: number;
  drawerResolvers?: InspectorDrawerResolver[];
  propertyDrawers?: InspectorPropertyDrawerInput;
  propertyDrawerResolvers?: InspectorPropertyDrawerResolver[];
  stateUpdaters?: InspectorStateUpdater[];
}

export type InspectorDrawerResolver = (
  property: InspectorProperty,
  context: InspectorPropertyTreeContext,
) => InspectorPropertyResolvedDrawer | null | undefined;

export type InspectorStateUpdater = (
  property: InspectorProperty,
  state: InspectorPropertyState,
  context: InspectorPropertyTreeContext,
) => Partial<InspectorPropertyState> | null | undefined;

export type InspectorPathSetInput =
  | Iterable<string>
  | Record<string, boolean | undefined | null>
  | null
  | undefined;

export type InspectorMessageInput =
  | Map<string, string>
  | Record<string, string | undefined | null>
  | null
  | undefined;

export interface InspectorPropertyTreeContext {
  id: string;
  targetId: string;
  searchQuery: string;
  readonly: boolean;
  disabled: boolean;
}

interface NormalizedTreeOptions extends InspectorPropertyTreeContext {
  selectedPath: string;
  expandedPaths: Set<string>;
  collapsedPaths: Set<string>;
  changedPaths: Set<string>;
  errors: Map<string, string>;
  warnings: Map<string, string>;
  defaultExpandedDepth: number;
  autoCollapseChildCount: number;
  drawerResolvers: InspectorDrawerResolver[];
  propertyDrawers: NormalizedPropertyDrawerRegistry;
  propertyDrawerResolvers: InspectorPropertyDrawerResolver[];
  stateUpdaters: InspectorStateUpdater[];
}

interface InspectorPropertyInit {
  snapshot: InspectorPropertySnapshot;
  parent: InspectorProperty | null;
  depth: number;
  index: number;
}

interface InspectorPropertyDrawerConfig {
  context: InspectorPropertyTreeContext;
  drawers: NormalizedPropertyDrawerRegistry;
  drawerResolvers: InspectorPropertyDrawerResolver[];
}

interface NormalizedPropertyDrawerRegistration {
  types: string[];
  valueTypes: string[];
  fieldTypes: string[];
  attributes: string[];
  propertyPaths: string[];
  names: string[];
  drawerKinds: string[];
  drawer: Component;
  match?: InspectorPropertyDrawerMatcher;
  priority: number;
  order: number;
}

interface NormalizedPropertyDrawerRegistry {
  entries: NormalizedPropertyDrawerRegistration[];
  libraries: InspectorPropertyDrawerLibrary[];
}

const VECTOR_TYPES = new Set(["Vector2", "Vector3", "Vector4", "Quaternion", "Rect"]);
const NUMBER_TYPES = new Set(["Integer", "ArraySize", "Float"]);
const DEFAULT_AUTO_COLLAPSE_CHILD_COUNT = 24;
const EMPTY_PROPERTY_DRAWER_REGISTRY: NormalizedPropertyDrawerRegistry = {
  entries: [],
  libraries: [],
};
const DEFAULT_DRAW_CONTEXT: InspectorPropertyTreeContext = {
  id: "property-tree",
  targetId: "",
  searchQuery: "",
  readonly: false,
  disabled: false,
};
const DEFAULT_PROPERTY_DRAWER_CONFIG: InspectorPropertyDrawerConfig = {
  context: DEFAULT_DRAW_CONTEXT,
  drawers: EMPTY_PROPERTY_DRAWER_REGISTRY,
  drawerResolvers: [],
};

export class InspectorProperty {
  readonly snapshot: InspectorPropertySnapshot;
  readonly parent: InspectorProperty | null;
  readonly depth: number;
  readonly index: number;
  readonly propertyPath: string;
  readonly id: string;
  readonly name: string;
  readonly label: string;
  readonly type: string;
  readonly valueType: string;
  readonly fieldTypeFullName: string;
  readonly fieldTypeAssembly: string;
  readonly value: unknown;
  readonly displayValue: string;
  readonly editable: boolean;
  readonly isArray: boolean;
  readonly arraySize: number;
  readonly isFlagsEnum: boolean;
  readonly enumValueIndex: number;
  readonly enumValueFlag: number;
  readonly enumOptions: InspectorSelectOption[];
  readonly isManagedReference: boolean;
  readonly managedReferenceFullTypename: string;
  readonly managedReferenceFieldTypename: string;
  readonly managedReferenceDisplayName: string;
  readonly managedReferenceTypes: InspectorManagedReferenceTypeOption[];
  readonly tooltip: string;
  readonly header: string;
  readonly hasRange: boolean;
  readonly rangeMin: number;
  readonly rangeMax: number;
  readonly numberStep: number;
  readonly multiline: boolean;
  readonly minLines: number;
  readonly maxLines: number;
  readonly referenceTypeFullName: string;
  readonly referenceTypeAssembly: string;
  readonly attributes: InspectorPropertyAttributeInfo[];
  children: InspectorProperty[] = [];
  drawer: InspectorPropertyResolvedDrawer = {
    kind: "unsupported",
    commitMode: "none",
    container: false,
    valueType: "Unsupported",
  };
  state: InspectorPropertyState = {
    editable: false,
    disabled: false,
    readonly: true,
    expanded: false,
    selected: false,
    changed: false,
    matchesSearch: false,
    hasVisibleDescendant: false,
    visible: true,
    error: "",
    warning: "",
  };
  private propertyDrawerConfig: InspectorPropertyDrawerConfig = DEFAULT_PROPERTY_DRAWER_CONFIG;

  constructor(init: InspectorPropertyInit) {
    this.snapshot = init.snapshot;
    this.parent = init.parent;
    this.depth = init.depth;
    this.index = init.index;
    this.propertyPath = init.snapshot.propertyPath;
    this.id = init.snapshot.propertyPath;
    this.name = init.snapshot.name || pathLeafName(init.snapshot.propertyPath) || `property-${init.index}`;
    this.label = init.snapshot.displayName || this.name || this.propertyPath;
    this.type = init.snapshot.type || init.snapshot.valueType || "Generic";
    this.valueType = init.snapshot.valueType || init.snapshot.type || "Generic";
    this.fieldTypeFullName = init.snapshot.fieldTypeFullName || "";
    this.fieldTypeAssembly = init.snapshot.fieldTypeAssembly || "";
    this.value = init.snapshot.value;
    this.displayValue = init.snapshot.displayValue || "";
    this.editable = init.snapshot.editable !== false;
    this.isArray = init.snapshot.isArray === true;
    this.arraySize = Number.isFinite(init.snapshot.arraySize) ? Number(init.snapshot.arraySize) : -1;
    this.isFlagsEnum = init.snapshot.isFlagsEnum === true;
    this.enumValueIndex = Number.isFinite(init.snapshot.enumValueIndex) ? Number(init.snapshot.enumValueIndex) : -1;
    this.enumValueFlag = Number.isFinite(init.snapshot.enumValueFlag) ? Number(init.snapshot.enumValueFlag) : 0;
    this.enumOptions = normalizeSelectOptions(init.snapshot.enumOptions);
    this.isManagedReference = init.snapshot.isManagedReference === true || this.valueType === "ManagedReference";
    this.managedReferenceFullTypename = init.snapshot.managedReferenceFullTypename || "";
    this.managedReferenceFieldTypename = init.snapshot.managedReferenceFieldTypename || "";
    this.managedReferenceDisplayName = init.snapshot.managedReferenceDisplayName || "";
    this.managedReferenceTypes = normalizeManagedReferenceTypes(
      init.snapshot.managedReferenceTypes,
      this.managedReferenceFullTypename,
      this.managedReferenceDisplayName,
    );
    this.tooltip = init.snapshot.tooltip || "";
    this.header = init.snapshot.header || "";
    this.hasRange = init.snapshot.hasRange === true;
    this.rangeMin = Number.isFinite(init.snapshot.rangeMin) ? Number(init.snapshot.rangeMin) : 0;
    this.rangeMax = Number.isFinite(init.snapshot.rangeMax) ? Number(init.snapshot.rangeMax) : 0;
    this.numberStep = Number.isFinite(init.snapshot.numberStep) ? Number(init.snapshot.numberStep) : 0;
    this.multiline = init.snapshot.multiline === true;
    this.minLines = Number.isFinite(init.snapshot.minLines) ? Number(init.snapshot.minLines) : 0;
    this.maxLines = Number.isFinite(init.snapshot.maxLines) ? Number(init.snapshot.maxLines) : 0;
    this.referenceTypeFullName = init.snapshot.referenceTypeFullName || "";
    this.referenceTypeAssembly = init.snapshot.referenceTypeAssembly || "";
    this.attributes = normalizePropertyAttributes(init.snapshot.attributes);
  }

  get hasChildren(): boolean {
    return this.children.length > 0 || this.snapshot.hasChildren === true;
  }

  get isLeaf(): boolean {
    return !this.hasChildren && !this.isArray && !this.isManagedReference;
  }

  get canEdit(): boolean {
    return this.state.editable && !this.state.disabled && !this.state.readonly;
  }

  get root(): InspectorProperty {
    let current: InspectorProperty = this;
    while (current.parent) current = current.parent;
    return current;
  }

  get searchText(): string {
    return [
      this.label,
      this.name,
      this.propertyPath,
      this.type,
      this.valueType,
      this.fieldTypeFullName,
      this.fieldTypeAssembly,
      this.displayValue,
      this.managedReferenceFullTypename,
      this.managedReferenceFieldTypename,
      this.managedReferenceDisplayName,
      ...this.managedReferenceTypes.flatMap((option) => [
        option.label,
        option.value,
        option.fullName,
        option.assembly,
      ]),
    ]
      .filter(Boolean)
      .join(" ")
      .toLowerCase();
  }

  get selectedManagedReferenceType(): InspectorManagedReferenceTypeOption | null {
    return resolveManagedReferenceTypeOption(this);
  }

  ancestors(): InspectorProperty[] {
    const result: InspectorProperty[] = [];
    let current = this.parent;
    while (current) {
      result.unshift(current);
      current = current.parent;
    }
    return result;
  }

  descendants(): InspectorProperty[] {
    const result: InspectorProperty[] = [];
    for (const child of this.children) {
      result.push(child, ...child.descendants());
    }
    return result;
  }

  flatten(options: InspectorPropertyTreeFlattenOptions = {}): InspectorProperty[] {
    const includeContainers = options.includeContainers !== false;
    const result: InspectorProperty[] = [];
    const visit = (property: InspectorProperty) => {
      if (options.visibleOnly && !property.state.visible) return;
      if (includeContainers || property.isLeaf) result.push(property);
      for (const child of property.children) visit(child);
    };
    visit(this);
    return result;
  }

  createCommit(value: unknown): InspectorPropertyCommit {
    return {
      propertyPath: this.propertyPath,
      value,
      property: this,
      snapshot: this.snapshot,
    };
  }

  draw(options: InspectorPropertyDrawerOptions = {}): VNodeChild {
    return drawInspectorProperty(this, options, this.propertyDrawerConfig);
  }

  propertyDrawerComponent(options: Pick<InspectorPropertyDrawerOptions, "drawer" | "drawers"> = {}): Component | null {
    return resolveInspectorPropertyDrawerComponent(this, options, this.propertyDrawerConfig);
  }

  hasPropertyDrawer(options: Pick<InspectorPropertyDrawerOptions, "drawer" | "drawers"> = {}): boolean {
    return this.propertyDrawerComponent(options) !== null;
  }

  searchManagedReferenceTypes(
    query = "",
    options: InspectorManagedReferenceTypeSearchOptions = {},
  ): InspectorManagedReferenceTypeOption[] {
    return searchManagedReferenceTypeOptions(this, query, options);
  }

  createManagedReferenceTypeCommit(
    type: string | InspectorManagedReferenceTypeOption | null | undefined,
  ): InspectorPropertyCommit {
    return this.createCommit(createManagedReferenceTypeCommand(type));
  }

  _setDrawConfig(config: InspectorPropertyDrawerConfig) {
    this.propertyDrawerConfig = config;
  }
}

export class PropertyTree {
  readonly id: string;
  readonly targetId: string;
  readonly searchQuery: string;
  readonly rootProperties: InspectorProperty[];
  readonly properties: InspectorProperty[];
  readonly byPath: Map<string, InspectorProperty>;
  private readonly sourceSnapshots: InspectorPropertySnapshot[];

  constructor(
    snapshots: InspectorPropertySnapshot | InspectorPropertySnapshot[] | null | undefined,
    options: InspectorPropertyTreeOptions = {},
  ) {
    const normalizedOptions = normalizeTreeOptions(options);
    this.id = normalizedOptions.id;
    this.targetId = normalizedOptions.targetId;
    this.searchQuery = normalizedOptions.searchQuery;
    this.sourceSnapshots = normalizeInputSnapshots(snapshots);
    this.rootProperties = this.sourceSnapshots.map((snapshot, index) =>
      buildInspectorProperty(snapshot, null, 0, index),
    );
    this.properties = this.rootProperties.flatMap((property) => property.flatten());
    this.byPath = new Map(this.properties.map((property) => [property.propertyPath, property]));
    applyDrawersAndState(this.properties, normalizedOptions);
  }

  get rootProperty(): InspectorProperty | null {
    return this.rootProperties[0] ?? null;
  }

  getProperty(propertyPath: string): InspectorProperty | null {
    return this.byPath.get(propertyPath) ?? null;
  }

  requireProperty(propertyPath: string): InspectorProperty {
    const property = this.getProperty(propertyPath);
    if (!property) throw new Error(`Inspector property not found: ${propertyPath}`);
    return property;
  }

  flatten(options: InspectorPropertyTreeFlattenOptions = {}): InspectorProperty[] {
    return this.rootProperties.flatMap((property) => property.flatten(options));
  }

  visibleProperties(options: Omit<InspectorPropertyTreeFlattenOptions, "visibleOnly"> = {}): InspectorProperty[] {
    return this.flatten({ ...options, visibleOnly: true });
  }

  search(query = this.searchQuery): InspectorProperty[] {
    const normalized = normalizeSearchQuery(query);
    if (!normalized) return this.properties;
    return this.properties.filter((property) => property.searchText.includes(normalized));
  }

  createCommit(propertyPath: string, value: unknown): InspectorPropertyCommit {
    return this.requireProperty(propertyPath).createCommit(value);
  }

  createManagedReferenceTypeCommit(
    propertyPath: string,
    type: string | InspectorManagedReferenceTypeOption | null | undefined,
  ): InspectorPropertyCommit {
    return this.requireProperty(propertyPath).createManagedReferenceTypeCommit(type);
  }

  draw(options: InspectorPropertyDrawerOptions = {}): VNodeChild {
    if (this.rootProperties.length === 0) return null;
    if (this.rootProperties.length === 1) return this.rootProperties[0].draw(options);
    return h(
      "div",
      {
        class: "inspector-property-tree-draw",
        "data-tree-id": this.id,
      },
      this.rootProperties.map((property) => property.draw(options)),
    );
  }

  withOptions(options: InspectorPropertyTreeOptions): PropertyTree {
    return new PropertyTree(this.sourceSnapshots, {
      id: this.id,
      targetId: this.targetId,
      ...options,
    });
  }
}

export function createPropertyTree(
  snapshots: InspectorPropertySnapshot | InspectorPropertySnapshot[] | null | undefined,
  options: InspectorPropertyTreeOptions = {},
): PropertyTree {
  return new PropertyTree(snapshots, options);
}

export function createInspectorPropertyTreeBinding(
  input: InspectorPropertyTreeBindingInput = {},
): InspectorPropertyTreeBinding {
  const id = input.id?.trim() || "property-tree";
  const targetId = input.targetId?.trim() || id;
  const loading = input.loading === true;
  return {
    id,
    targetId,
    snapshots: input.snapshots ?? null,
    loading,
    error: input.error || "",
    disabled: input.disabled === true || loading,
    readonly: input.readonly === true,
    editable: input.editable !== false,
    commit: input.commit ?? noopInspectorPropertyTreeCommit,
  };
}

export function resolveInspectorDrawer(property: InspectorProperty): InspectorPropertyResolvedDrawer {
  if (property.isArray) return drawer("array", "command", property.valueType, true);
  if (property.isManagedReference) return drawer("managedReference", "command", property.valueType, true);

  const valueType = property.valueType || property.type;
  if (valueType === "Boolean") return drawer("boolean", "change", valueType);
  if (valueType === "Enum") return drawer(property.isFlagsEnum ? "flags" : "enum", "change", valueType);
  if (valueType === "LayerMask") return drawer("layerMask", "blur", valueType);
  if (NUMBER_TYPES.has(valueType)) return drawer("number", "blur", valueType);
  if (VECTOR_TYPES.has(valueType)) return drawer("vector", "blur", valueType);
  if (valueType === "Color") return drawer("color", "change", valueType);
  if (valueType === "ObjectReference") return drawer("objectReference", "blur", valueType);
  if (valueType === "String") return drawer("text", "blur", valueType);
  if (property.children.length > 0 || property.snapshot.hasChildren) {
    return drawer("object", "none", property.valueType, true);
  }
  return drawer(property.editable ? "text" : "unsupported", property.editable ? "blur" : "none", valueType);
}

export function resolveManagedReferenceTypeOption(
  property: InspectorProperty,
  typeName = property.managedReferenceFullTypename,
): InspectorManagedReferenceTypeOption | null {
  const normalizedTypeName = typeName.trim();
  if (!normalizedTypeName) return null;
  return property.managedReferenceTypes.find((option) => option.value === normalizedTypeName) ?? {
    label: managedReferenceDisplayName(normalizedTypeName),
    value: normalizedTypeName,
    current: true,
    unavailable: true,
    ...splitManagedReferenceTypeName(normalizedTypeName),
  };
}

export function searchManagedReferenceTypeOptions(
  property: InspectorProperty,
  query = "",
  options: InspectorManagedReferenceTypeSearchOptions = {},
): InspectorManagedReferenceTypeOption[] {
  const normalizedQuery = normalizeSearchQuery(query);
  const limit = Math.max(0, options.limit ?? 80);
  const results = normalizedQuery
    ? property.managedReferenceTypes.filter((option) =>
      managedReferenceTypeSearchText(option).includes(normalizedQuery),
    )
    : property.managedReferenceTypes;
  return limit > 0 ? results.slice(0, limit) : results;
}

export function defineInspectorPropertyDrawers(
  input: InspectorPropertyDrawerInput,
): InspectorPropertyDrawerRegistration[] {
  return expandPropertyDrawerRegistrations(input).map((entry) => ({
    ...entry,
    type: normalizeDrawerTypes(entry.type),
    valueType: normalizeDrawerTypes(entry.valueType),
    fieldType: normalizeDrawerTypes(entry.fieldType),
    attribute: normalizeDrawerTypes(entry.attribute),
    propertyPath: normalizeDrawerTypes(entry.propertyPath),
    name: normalizeDrawerTypes(entry.name),
    drawerKind: normalizeDrawerTypes(entry.drawerKind),
  }));
}

export function createInspectorPropertyDrawerLibrary(
  input?: InspectorPropertyDrawerInput,
): InspectorPropertyDrawerLibrary {
  const library = new MutableInspectorPropertyDrawerLibrary();
  for (const registration of expandPropertyDrawerRegistrations(input)) {
    library.register(registration);
  }
  return library;
}

class MutableInspectorPropertyDrawerLibrary implements InspectorPropertyDrawerLibrary {
  private readonly registeredDrawers: InspectorPropertyDrawerRegistration[] = [];

  get registrations(): readonly InspectorPropertyDrawerRegistration[] {
    return this.registeredDrawers;
  }

  register(registration: InspectorPropertyDrawerRegistration): () => void;
  register(
    type: string | string[],
    drawer: Component,
    options?: Omit<InspectorPropertyDrawerRegistration, "type" | "drawer">,
  ): () => void;
  register(
    registrationOrType: InspectorPropertyDrawerRegistration | string | string[],
    drawer?: Component,
    options: Omit<InspectorPropertyDrawerRegistration, "type" | "drawer"> = {},
  ): () => void {
    if (typeof registrationOrType === "string" || Array.isArray(registrationOrType)) {
      if (!drawer) return () => undefined;
      return this.register({
        ...options,
        type: registrationOrType,
        drawer,
      });
    }
    const registration = registrationOrType;
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

  resolve(property: InspectorProperty, context: InspectorPropertyTreeContext): Component | null {
    return findPropertyDrawerComponent(
      property,
      context,
      normalizePropertyDrawers(this.registeredDrawers),
    );
  }
}

export const publicInspectorPropertyDrawerLibrary = createInspectorPropertyDrawerLibrary();
export const projectInspectorPropertyDrawerLibrary = publicInspectorPropertyDrawerLibrary;

export function normalizeInspectorPropertyDrawers(
  input: InspectorPropertyDrawerInput,
): InspectorPropertyDrawerRegistration[] {
  return defineInspectorPropertyDrawers(input);
}

export function registerInspectorPropertyDrawer(
  type: string | string[],
  drawer: Component,
  options: Omit<InspectorPropertyDrawerRegistration, "type" | "drawer"> = {},
): () => void {
  return publicInspectorPropertyDrawerLibrary.register(type, drawer, options);
}

export function registerInspectorValueDrawer(
  valueType: string | string[],
  drawer: Component,
  options: Omit<InspectorPropertyDrawerRegistration, "valueType" | "drawer"> = {},
): () => void {
  return publicInspectorPropertyDrawerLibrary.register({
    ...options,
    valueType,
    drawer,
  });
}

export function registerInspectorFieldDrawer(
  fieldType: string | string[],
  drawer: Component,
  options: Omit<InspectorPropertyDrawerRegistration, "fieldType" | "drawer"> = {},
): () => void {
  return publicInspectorPropertyDrawerLibrary.register({
    ...options,
    fieldType,
    drawer,
  });
}

export function registerInspectorAttributeDrawer(
  attribute: string | string[],
  drawer: Component,
  options: Omit<InspectorPropertyDrawerRegistration, "attribute" | "drawer"> = {},
): () => void {
  return publicInspectorPropertyDrawerLibrary.register({
    ...options,
    attribute,
    drawer,
  });
}

export function registerInspectorPropertyPathDrawer(
  propertyPath: string | string[],
  drawer: Component,
  options: Omit<InspectorPropertyDrawerRegistration, "propertyPath" | "drawer"> = {},
): () => void {
  return publicInspectorPropertyDrawerLibrary.register({
    ...options,
    propertyPath,
    drawer,
  });
}

export const propertyTreeService = {
  createTree: createPropertyTree,
  createBinding: createInspectorPropertyTreeBinding,
  resolveDrawer: resolveInspectorDrawer,
  resolveManagedReferenceTypeOption,
  searchManagedReferenceTypeOptions,
  definePropertyDrawers: defineInspectorPropertyDrawers,
  createPropertyDrawerLibrary: createInspectorPropertyDrawerLibrary,
  publicPropertyDrawerLibrary: publicInspectorPropertyDrawerLibrary,
  projectPropertyDrawerLibrary: projectInspectorPropertyDrawerLibrary,
  normalizePropertyDrawers: normalizeInspectorPropertyDrawers,
  registerPropertyDrawer: registerInspectorPropertyDrawer,
  registerValueDrawer: registerInspectorValueDrawer,
  registerFieldDrawer: registerInspectorFieldDrawer,
  registerAttributeDrawer: registerInspectorAttributeDrawer,
  registerPropertyPathDrawer: registerInspectorPropertyPathDrawer,
};

function noopInspectorPropertyTreeCommit() {
  return undefined;
}

function buildInspectorProperty(
  source: InspectorPropertySnapshot,
  parent: InspectorProperty | null,
  depth: number,
  index: number,
): InspectorProperty {
  const snapshot = normalizeSnapshot(source, parent, index);
  const property = new InspectorProperty({ snapshot, parent, depth, index });
  property.children = (source.children ?? []).map((child, childIndex) =>
    buildInspectorProperty(child, property, depth + 1, childIndex),
  );
  return property;
}

function normalizeSnapshot(
  source: InspectorPropertySnapshot,
  parent: InspectorProperty | null,
  index: number,
): InspectorPropertySnapshot {
  const fallbackName = source.name || source.displayName || `property-${index}`;
  const propertyPath = source.propertyPath || pathForChild(parent, fallbackName, index);
  return {
    ...source,
    propertyPath,
    name: source.name || pathLeafName(propertyPath) || fallbackName,
    displayName: source.displayName || source.name || pathLeafName(propertyPath) || fallbackName,
    type: source.type || source.valueType || "Generic",
    valueType: source.valueType || source.type || "Generic",
    fieldTypeFullName: source.fieldTypeFullName || "",
    fieldTypeAssembly: source.fieldTypeAssembly || "",
    tooltip: source.tooltip || "",
    header: source.header || "",
    hasRange: source.hasRange === true,
    rangeMin: Number.isFinite(source.rangeMin) ? Number(source.rangeMin) : 0,
    rangeMax: Number.isFinite(source.rangeMax) ? Number(source.rangeMax) : 0,
    numberStep: Number.isFinite(source.numberStep) ? Number(source.numberStep) : 0,
    multiline: source.multiline === true,
    minLines: Number.isFinite(source.minLines) ? Number(source.minLines) : 0,
    maxLines: Number.isFinite(source.maxLines) ? Number(source.maxLines) : 0,
    referenceTypeFullName: source.referenceTypeFullName || "",
    referenceTypeAssembly: source.referenceTypeAssembly || "",
    attributes: normalizePropertyAttributes(source.attributes),
    displayValue: source.displayValue || "",
    editable: source.editable !== false,
    hasChildren: source.hasChildren === true || Boolean(source.children?.length),
    isArray: source.isArray === true,
    arraySize: Number.isFinite(source.arraySize) ? Number(source.arraySize) : -1,
    isFlagsEnum: source.isFlagsEnum === true,
    enumValueIndex: Number.isFinite(source.enumValueIndex) ? Number(source.enumValueIndex) : -1,
    enumValueFlag: Number.isFinite(source.enumValueFlag) ? Number(source.enumValueFlag) : 0,
    enumOptions: normalizeSelectOptions(source.enumOptions),
    children: Array.isArray(source.children) ? source.children : [],
    isManagedReference: source.isManagedReference === true || source.valueType === "ManagedReference",
    managedReferenceFullTypename: source.managedReferenceFullTypename || "",
    managedReferenceFieldTypename: source.managedReferenceFieldTypename || "",
    managedReferenceDisplayName: source.managedReferenceDisplayName || "",
    managedReferenceTypes: normalizeManagedReferenceTypes(
      source.managedReferenceTypes,
      source.managedReferenceFullTypename || "",
      source.managedReferenceDisplayName || "",
    ),
  };
}

function applyDrawersAndState(properties: InspectorProperty[], options: NormalizedTreeOptions) {
  const context: InspectorPropertyTreeContext = {
    id: options.id,
    targetId: options.targetId,
    searchQuery: options.searchQuery,
    readonly: options.readonly,
    disabled: options.disabled,
  };
  const propertyDrawerConfig: InspectorPropertyDrawerConfig = {
    context,
    drawers: options.propertyDrawers,
    drawerResolvers: options.propertyDrawerResolvers,
  };
  for (const property of properties) {
    property._setDrawConfig(propertyDrawerConfig);
    property.drawer = resolveDrawerWithResolvers(property, context, options.drawerResolvers);
    property.state = createBaseState(property, options);
  }

  applySearchVisibility(properties, options.searchQuery);

  for (const property of properties) {
    let nextState = property.state;
    for (const updater of options.stateUpdaters) {
      nextState = {
        ...nextState,
        ...(updater(property, nextState, context) ?? {}),
      };
    }
    property.state = nextState;
  }
}

function resolveDrawerWithResolvers(
  property: InspectorProperty,
  context: InspectorPropertyTreeContext,
  resolvers: InspectorDrawerResolver[],
): InspectorPropertyResolvedDrawer {
  for (const resolver of resolvers) {
    const resolved = resolver(property, context);
    if (resolved) return resolved;
  }
  return resolveInspectorDrawer(property);
}

function createBaseState(
  property: InspectorProperty,
  options: NormalizedTreeOptions,
): InspectorPropertyState {
  const path = property.propertyPath;
  const childCount = property.children.length;
  const expandedByDefault =
    property.depth < options.defaultExpandedDepth &&
    childCount > 0 &&
    childCount <= options.autoCollapseChildCount;
  const expanded = options.collapsedPaths.has(path)
    ? false
    : options.expandedPaths.has(path) || expandedByDefault;

  return {
    editable: property.editable,
    disabled: options.disabled,
    readonly: options.readonly || !property.editable,
    expanded,
    selected: path === options.selectedPath,
    changed: options.changedPaths.has(path),
    matchesSearch: false,
    hasVisibleDescendant: false,
    visible: true,
    error: options.errors.get(path) ?? "",
    warning: options.warnings.get(path) ?? "",
  };
}

function applySearchVisibility(properties: InspectorProperty[], searchQuery: string) {
  if (!searchQuery) {
    for (const property of properties) {
      property.state = {
        ...property.state,
        matchesSearch: false,
        hasVisibleDescendant: false,
        visible: true,
      };
    }
    return;
  }

  for (const property of properties) {
    property.state = {
      ...property.state,
      matchesSearch: property.searchText.includes(searchQuery),
      hasVisibleDescendant: false,
      visible: false,
    };
  }

  for (let index = properties.length - 1; index >= 0; index -= 1) {
    const property = properties[index];
    const hasVisibleDescendant = property.children.some((child) => child.state.visible);
    property.state = {
      ...property.state,
      hasVisibleDescendant,
      visible: property.state.matchesSearch || hasVisibleDescendant,
    };
  }
}

function normalizeTreeOptions(options: InspectorPropertyTreeOptions): NormalizedTreeOptions {
  return {
    id: options.id?.trim() || "property-tree",
    targetId: options.targetId?.trim() || "",
    searchQuery: normalizeSearchQuery(options.searchQuery || ""),
    readonly: options.readonly === true,
    disabled: options.disabled === true,
    selectedPath: options.selectedPath?.trim() || "",
    expandedPaths: normalizePathSet(options.expandedPaths),
    collapsedPaths: normalizePathSet(options.collapsedPaths),
    changedPaths: normalizePathSet(options.changedPaths),
    errors: normalizeMessageMap(options.errors),
    warnings: normalizeMessageMap(options.warnings),
    defaultExpandedDepth: Math.max(0, options.defaultExpandedDepth ?? 1),
    autoCollapseChildCount: Math.max(0, options.autoCollapseChildCount ?? DEFAULT_AUTO_COLLAPSE_CHILD_COUNT),
    drawerResolvers: options.drawerResolvers ?? [],
    propertyDrawers: normalizePropertyDrawers(options.propertyDrawers),
    propertyDrawerResolvers: options.propertyDrawerResolvers ?? [],
    stateUpdaters: options.stateUpdaters ?? [],
  };
}

function drawInspectorProperty(
  property: InspectorProperty,
  options: InspectorPropertyDrawerOptions,
  config: InspectorPropertyDrawerConfig,
): VNodeChild {
  const component = resolveInspectorPropertyDrawerComponent(property, options, config);
  if (!component) return drawInspectorPropertyFallback(property, options);

  const disabled = options.disabled ?? property.state.disabled;
  const readonly = options.readonly ?? property.state.readonly;
  const compact = options.compact === true;
  const commit = (value: unknown, targetProperty = property) => {
    const targetDisabled = options.disabled ?? targetProperty.state.disabled;
    const targetReadonly = options.readonly ?? targetProperty.state.readonly;
    if (targetDisabled || targetReadonly || !targetProperty.editable) return;
    options.onCommit?.(normalizeDrawCommit(targetProperty, value));
  };
  const drawOptions = childDrawOptions(options);
  const draw = (targetProperty = property, nextOptions: InspectorPropertyDrawerOptions = {}) =>
    targetProperty.draw({ ...drawOptions, ...nextOptions });
  const drawChild = (
    propertyOrPath: InspectorProperty | string,
    nextOptions: InspectorPropertyDrawerOptions = {},
  ) => {
    const child = typeof propertyOrPath === "string"
      ? findDrawChild(property, propertyOrPath)
      : propertyOrPath;
    return child ? draw(child, nextOptions) : null;
  };

  return h(component, {
    property,
    snapshot: property.snapshot,
    propertyPath: property.propertyPath,
    label: property.label,
    value: property.value,
    modelValue: property.value,
    displayValue: property.displayValue,
    type: property.type,
    valueType: property.valueType,
    propertyType: property.valueType,
    fieldTypeFullName: property.fieldTypeFullName,
    fieldTypeAssembly: property.fieldTypeAssembly,
    tooltip: property.tooltip,
    header: property.header,
    hasRange: property.hasRange,
    rangeMin: property.rangeMin,
    rangeMax: property.rangeMax,
    numberStep: property.numberStep,
    multiline: property.multiline,
    minLines: property.minLines,
    maxLines: property.maxLines,
    referenceTypeFullName: property.referenceTypeFullName,
    referenceTypeAssembly: property.referenceTypeAssembly,
    attributes: property.attributes,
    editable: property.editable,
    disabled,
    readonly,
    compact,
    treeId: config.context.id,
    targetId: config.context.targetId,
    depth: property.depth,
    children: property.children,
    commit,
    draw,
    drawChild,
    onCommit: commit,
  });
}

function resolveInspectorPropertyDrawerComponent(
  property: InspectorProperty,
  options: Pick<InspectorPropertyDrawerOptions, "drawer" | "drawers">,
  config: InspectorPropertyDrawerConfig,
): Component | null {
  if (options.drawer) return options.drawer;
  const optionComponent = findPropertyDrawerComponent(
    property,
    config.context,
    normalizePropertyDrawers(options.drawers),
  );
  if (optionComponent) return optionComponent;

  for (const resolver of config.drawerResolvers) {
    const resolved = resolver(property, config.context);
    if (resolved) return resolved;
  }

  return findPropertyDrawerComponent(property, config.context, config.drawers)
    ?? findPropertyDrawerComponent(
      property,
      config.context,
      normalizePropertyDrawers(publicInspectorPropertyDrawerLibrary),
    );
}

function normalizePropertyDrawers(
  input: InspectorPropertyDrawerInput,
): NormalizedPropertyDrawerRegistry {
  if (!input) return EMPTY_PROPERTY_DRAWER_REGISTRY;
  if (isInspectorPropertyDrawerLibrary(input)) {
    return {
      entries: [],
      libraries: [input],
    };
  }
  const entries: NormalizedPropertyDrawerRegistration[] = [];
  let order = 0;
  const pushRegistration = (registration: InspectorPropertyDrawerRegistration) => {
    if (!registration.drawer) return;
    const types = normalizeDrawerTypes(registration.type);
    const valueTypes = normalizeDrawerTypes(registration.valueType);
    const fieldTypes = normalizeDrawerTypes(registration.fieldType);
    const attributes = normalizeDrawerTypes(registration.attribute);
    const propertyPaths = normalizeDrawerTypes(registration.propertyPath);
    const names = normalizeDrawerTypes(registration.name);
    const drawerKinds = normalizeDrawerTypes(registration.drawerKind);
    if (
      !types.length &&
      !valueTypes.length &&
      !fieldTypes.length &&
      !attributes.length &&
      !propertyPaths.length &&
      !names.length &&
      !drawerKinds.length &&
      !registration.match
    ) return;
    entries.push({
      types,
      valueTypes,
      fieldTypes,
      attributes,
      propertyPaths,
      names,
      drawerKinds,
      drawer: registration.drawer,
      match: registration.match,
      priority: Number.isFinite(registration.priority) ? Number(registration.priority) : 0,
      order,
    });
    order += 1;
  };

  for (const registration of expandPropertyDrawerRegistrations(input)) {
    pushRegistration(registration);
  }

  entries.sort((left, right) => right.priority - left.priority || left.order - right.order);
  return entries.length ? { entries, libraries: [] } : EMPTY_PROPERTY_DRAWER_REGISTRY;
}

function expandPropertyDrawerRegistrations(
  input: InspectorPropertyDrawerInput,
): InspectorPropertyDrawerRegistration[] {
  if (!input) return [];
  if (isInspectorPropertyDrawerLibrary(input)) return [...input.registrations];
  if (Array.isArray(input)) return [...input];
  if (input instanceof Map) {
    return Array.from(input.entries()).map(([type, drawer]) => ({ type, drawer }));
  }
  return Object.entries(input).map(([type, drawer]) => ({ type, drawer }));
}

function isInspectorPropertyDrawerLibrary(value: unknown): value is InspectorPropertyDrawerLibrary {
  return Boolean(
    value &&
    typeof value === "object" &&
    "registrations" in value &&
    "register" in value &&
    "resolve" in value,
  );
}

function normalizeDrawerTypes(type: string | string[] | undefined): string[] {
  return (Array.isArray(type) ? type : [type])
    .map((value) => normalizeDrawTypeKey(value || ""))
    .filter(Boolean);
}

function normalizePropertyAttributes(
  attributes: InspectorPropertyAttributeInfo[] | null | undefined,
): InspectorPropertyAttributeInfo[] {
  return (attributes ?? [])
    .map((attribute) => ({
      type: attribute.type || "",
      displayName: attribute.displayName || attribute.type || "",
      value: attribute.value || "",
    }))
    .filter((attribute) => attribute.type || attribute.displayName || attribute.value);
}

function findPropertyDrawerComponent(
  property: InspectorProperty,
  context: InspectorPropertyTreeContext,
  registry: NormalizedPropertyDrawerRegistry,
): Component | null {
  if (!registry.entries.length && !registry.libraries.length) return null;
  const keys = propertyDrawerLookupKeys(property);
  const valueType = normalizeDrawTypeKey(property.valueType);
  const fieldType = normalizeDrawTypeKey(property.fieldTypeFullName || property.fieldTypeAssembly);
  const attributes = propertyAttributeLookupKeys(property);
  const propertyPath = normalizeDrawTypeKey(property.propertyPath);
  const name = normalizeDrawTypeKey(property.name);
  const drawerKind = normalizeDrawTypeKey(property.drawer.kind);
  for (const entry of registry.entries) {
    if (
      entryPropertyDrawerMatches(entry, {
        property,
        context,
        keys,
        valueType,
        fieldType,
        attributes,
        propertyPath,
        name,
        drawerKind,
      })
    ) return entry.drawer;
  }
  for (const library of registry.libraries) {
    const resolved = library.resolve(property, context);
    if (resolved) return resolved;
  }
  return null;
}

function entryPropertyDrawerMatches(
  entry: NormalizedPropertyDrawerRegistration,
  target: {
    property: InspectorProperty;
    context: InspectorPropertyTreeContext;
    keys: Set<string>;
    valueType: string;
    fieldType: string;
    attributes: Set<string>;
    propertyPath: string;
    name: string;
    drawerKind: string;
  },
): boolean {
  if (entry.match && !entry.match(target.property, target.context)) return false;
  if (entry.types.length && !entry.types.some((type) => type === "*" || target.keys.has(type))) return false;
  if (
    entry.valueTypes.length &&
    !entry.valueTypes.some((type) => type === "*" || type === target.valueType || target.keys.has(type))
  ) return false;
  if (
    entry.fieldTypes.length &&
    !entry.fieldTypes.some((type) => type === "*" || type === target.fieldType || target.keys.has(type))
  ) return false;
  if (entry.attributes.length && !entry.attributes.some((type) => type === "*" || target.attributes.has(type))) {
    return false;
  }
  if (entry.propertyPaths.length && !entry.propertyPaths.some((path) => path === "*" || path === target.propertyPath)) {
    return false;
  }
  if (entry.names.length && !entry.names.some((item) => item === "*" || item === target.name)) return false;
  if (entry.drawerKinds.length && !entry.drawerKinds.some((kind) => kind === "*" || kind === target.drawerKind)) {
    return false;
  }
  return true;
}

function propertyDrawerLookupKeys(property: InspectorProperty): Set<string> {
  return new Set([
    property.valueType,
    property.type,
    property.fieldTypeFullName,
    property.fieldTypeAssembly,
    property.referenceTypeFullName,
    property.referenceTypeAssembly,
    property.drawer.kind,
    property.propertyPath,
    property.name,
    property.managedReferenceFullTypename,
    property.managedReferenceFieldTypename,
    property.managedReferenceDisplayName,
    ...property.attributes.flatMap((attribute) => [
      attribute.type || "",
      attribute.displayName || "",
    ]),
  ]
    .map(normalizeDrawTypeKey)
    .filter(Boolean));
}

function propertyAttributeLookupKeys(property: InspectorProperty): Set<string> {
  return new Set(property.attributes
    .flatMap((attribute) => [
      attribute.type || "",
      attribute.displayName || "",
      shortTypeName(attribute.type || ""),
    ])
    .map(normalizeDrawTypeKey)
    .filter(Boolean));
}

function shortTypeName(value: string): string {
  const normalized = value.trim();
  const dot = normalized.lastIndexOf(".");
  return dot >= 0 ? normalized.slice(dot + 1) : normalized;
}

function normalizeDrawTypeKey(value: string): string {
  return value.trim().toLowerCase();
}

function drawInspectorPropertyFallback(
  property: InspectorProperty,
  options: InspectorPropertyDrawerOptions,
): VNodeChild {
  const showLabel = options.showLabel !== false;
  if (property.children.length > 0) {
    return h(
      "div",
      {
        class: [
          "inspector-property-draw-group",
          options.compact === true ? "compact" : "",
        ],
        "data-property-path": property.propertyPath,
      },
      [
        showLabel
          ? h("div", { class: "inspector-property-draw-header" }, [
              h("span", { class: "inspector-property-draw-label" }, property.label),
              h("span", { class: "inspector-property-draw-type" }, property.valueType),
            ])
          : null,
        h(
          "div",
          { class: "inspector-property-draw-children" },
          property.children.map((child) => child.draw(childDrawOptions(options))),
        ),
      ],
    );
  }

  return h(
    "label",
    {
      class: [
        "inspector-property-draw-row",
        options.compact === true ? "compact" : "",
      ],
      "data-property-path": property.propertyPath,
    },
    [
      showLabel ? h("span", { class: "inspector-property-draw-label" }, property.label) : null,
      h("span", { class: "inspector-property-draw-value" }, stringifyDrawValue(property)),
    ],
  );
}

function childDrawOptions(options: InspectorPropertyDrawerOptions): InspectorPropertyDrawerOptions {
  return {
    drawers: options.drawers,
    disabled: options.disabled,
    readonly: options.readonly,
    compact: options.compact,
    showLabel: options.showLabel,
    onCommit: options.onCommit,
  };
}

function findDrawChild(property: InspectorProperty, propertyPathOrName: string): InspectorProperty | null {
  const normalized = propertyPathOrName.trim();
  if (!normalized) return null;
  return property.children.find((child) =>
    child.propertyPath === normalized ||
    child.name === normalized ||
    child.label === normalized,
  ) ?? property.descendants().find((child) => child.propertyPath === normalized) ?? null;
}

function normalizeDrawCommit(property: InspectorProperty, value: unknown): InspectorPropertyCommit {
  if (isDrawCommitLike(value)) {
    const snapshot = isPropertySnapshotLike(value.snapshot)
      ? value.snapshot
      : isPropertySnapshotLike(value.property)
        ? value.property
        : property.snapshot;
    return {
      propertyPath: String(value.propertyPath),
      value: value.value,
      property: value.property instanceof InspectorProperty ? value.property : property,
      snapshot,
    };
  }
  return property.createCommit(value);
}

function isDrawCommitLike(value: unknown): value is {
  propertyPath: unknown;
  value: unknown;
  property?: unknown;
  snapshot?: unknown;
} {
  return Boolean(
    value &&
    typeof value === "object" &&
    "propertyPath" in value &&
    "value" in value,
  );
}

function isPropertySnapshotLike(value: unknown): value is InspectorPropertySnapshot {
  return Boolean(value && typeof value === "object" && "propertyPath" in value);
}

function stringifyDrawValue(property: InspectorProperty): string {
  if (property.displayValue) return property.displayValue;
  if (property.value == null) return "";
  if (typeof property.value === "object") return JSON.stringify(property.value);
  return String(property.value);
}

function normalizeInputSnapshots(
  snapshots: InspectorPropertySnapshot | InspectorPropertySnapshot[] | null | undefined,
): InspectorPropertySnapshot[] {
  if (!snapshots) return [];
  return Array.isArray(snapshots) ? snapshots : [snapshots];
}

function normalizePathSet(input: InspectorPathSetInput): Set<string> {
  const result = new Set<string>();
  if (!input) return result;
  if (isIterable(input)) {
    for (const path of input) {
      const normalized = String(path || "").trim();
      if (normalized) result.add(normalized);
    }
    return result;
  }
  for (const [path, enabled] of Object.entries(input)) {
    if (enabled) result.add(path);
  }
  return result;
}

function normalizeMessageMap(input: InspectorMessageInput): Map<string, string> {
  const result = new Map<string, string>();
  if (!input) return result;
  if (input instanceof Map) {
    for (const [path, message] of input) {
      if (path && message) result.set(path, message);
    }
    return result;
  }
  for (const [path, message] of Object.entries(input)) {
    if (path && message) result.set(path, message);
  }
  return result;
}

function normalizeSelectOptions(options: InspectorSelectOptionInput[] | undefined): InspectorSelectOption[] {
  if (!Array.isArray(options)) return [];
  return options.map((option) => {
    const index = Number.isFinite(option.index) ? Number(option.index) : undefined;
    const numericValue = Number.isFinite(option.numericValue) ? Number(option.numericValue) : undefined;
    const fallbackValue = index != null ? String(index) : option.name || option.label || "";
    const value = option.value || fallbackValue;
    return {
      label: option.label || option.name || value,
      value,
      name: option.name,
      index,
      numericValue,
    };
  });
}

function normalizeManagedReferenceTypes(
  options: InspectorManagedReferenceTypeOption[] | undefined,
  currentTypeName: string,
  currentDisplayName: string,
): InspectorManagedReferenceTypeOption[] {
  const result: InspectorManagedReferenceTypeOption[] = [];
  const seen = new Set<string>();
  for (const option of options ?? []) {
    const normalized = normalizeManagedReferenceTypeOption(option);
    if (!normalized.value || seen.has(normalized.value)) continue;
    seen.add(normalized.value);
    result.push({
      ...normalized,
      current: normalized.value === currentTypeName,
    });
  }

  const current = currentTypeName.trim();
  if (current && !seen.has(current)) {
    result.unshift({
      label: currentDisplayName || managedReferenceDisplayName(current),
      value: current,
      current: true,
      unavailable: true,
      ...splitManagedReferenceTypeName(current),
    });
  }
  return result;
}

function normalizeManagedReferenceTypeOption(
  option: InspectorManagedReferenceTypeOption,
): InspectorManagedReferenceTypeOption {
  const value = (option.value || combineManagedReferenceTypeName(option.assembly, option.fullName)).trim();
  const split = splitManagedReferenceTypeName(value);
  return {
    label: option.label || option.fullName || managedReferenceDisplayName(value) || value,
    value,
    fullName: option.fullName || split.fullName,
    assembly: option.assembly || split.assembly,
    current: option.current === true,
    unavailable: option.unavailable === true,
  };
}

function createManagedReferenceTypeCommand(
  type: string | InspectorManagedReferenceTypeOption | null | undefined,
): { action: "clear" } | {
  action: "setType";
  typeName: string;
  fullName?: string;
  assembly?: string;
} {
  if (!type) return { action: "clear" };
  if (typeof type === "string") {
    const typeName = type.trim();
    return typeName ? { action: "setType", typeName } : { action: "clear" };
  }
  const typeName = type.value.trim();
  if (!typeName) return { action: "clear" };
  return {
    action: "setType",
    typeName,
    fullName: type.fullName,
    assembly: type.assembly,
  };
}

function managedReferenceTypeSearchText(option: InspectorManagedReferenceTypeOption): string {
  return [
    option.label,
    option.value,
    option.fullName,
    option.assembly,
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
}

function managedReferenceDisplayName(typeName: string): string {
  const fullName = splitManagedReferenceTypeName(typeName).fullName || typeName.trim();
  const dot = fullName.lastIndexOf(".");
  return dot >= 0 ? fullName.slice(dot + 1) : fullName;
}

function combineManagedReferenceTypeName(
  assembly: string | null | undefined,
  fullName: string | null | undefined,
): string {
  const normalizedFullName = (fullName || "").trim();
  if (!normalizedFullName) return "";
  const normalizedAssembly = (assembly || "").trim();
  return normalizedAssembly ? `${normalizedAssembly} ${normalizedFullName}` : normalizedFullName;
}

function splitManagedReferenceTypeName(typeName: string): { assembly: string; fullName: string } {
  const normalized = typeName.trim();
  const space = normalized.indexOf(" ");
  if (space > 0) {
    return {
      assembly: normalized.slice(0, space).trim(),
      fullName: normalized.slice(space + 1).trim(),
    };
  }
  const comma = normalized.indexOf(",");
  if (comma > 0) {
    return {
      fullName: normalized.slice(0, comma).trim(),
      assembly: normalized.slice(comma + 1).trim().split(",")[0].trim(),
    };
  }
  return {
    assembly: "",
    fullName: normalized,
  };
}

function normalizeSearchQuery(query: string): string {
  return query.trim().toLowerCase();
}

function drawer(
  kind: InspectorDrawerKind,
  commitMode: InspectorCommitMode,
  valueType: string,
  container = false,
): InspectorPropertyResolvedDrawer {
  return {
    kind,
    commitMode,
    container,
    valueType,
  };
}

function pathForChild(parent: InspectorProperty | null, fallbackName: string, index: number): string {
  if (!parent) return fallbackName || `property-${index}`;
  if (parent.isArray) return `${parent.propertyPath}.Array.data[${index}]`;
  return `${parent.propertyPath}.${fallbackName || `property-${index}`}`;
}

function pathLeafName(path: string): string {
  const normalized = path.trim();
  if (!normalized) return "";
  const arrayMatch = normalized.match(/Array\.data\[(\d+)\]$/);
  if (arrayMatch) return `[${arrayMatch[1]}]`;
  const dot = normalized.lastIndexOf(".");
  return dot >= 0 ? normalized.slice(dot + 1) : normalized;
}

function isIterable(value: unknown): value is Iterable<string> {
  return Boolean(value && typeof value === "object" && Symbol.iterator in value);
}
