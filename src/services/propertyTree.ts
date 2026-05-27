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
}

export interface InspectorPropertyDrawer {
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

export type InspectorPropertyDrawMatcher = (
  property: InspectorProperty,
  context: InspectorPropertyTreeContext,
) => boolean;

export type InspectorPropertyDrawComponentResolver = (
  property: InspectorProperty,
  context: InspectorPropertyTreeContext,
) => Component | null | undefined;

export interface InspectorPropertyDrawComponentRegistration {
  type?: string | string[];
  component: Component;
  match?: InspectorPropertyDrawMatcher;
  priority?: number;
}

export interface InspectorPropertyDrawLibrary {
  readonly registrations: readonly InspectorPropertyDrawComponentRegistration[];
  register(registration: InspectorPropertyDrawComponentRegistration): () => void;
  register(
    type: string | string[],
    component: Component,
    options?: Omit<InspectorPropertyDrawComponentRegistration, "type" | "component">,
  ): () => void;
  clear(): void;
  resolve(property: InspectorProperty, context: InspectorPropertyTreeContext): Component | null;
}

export type InspectorPropertyDrawComponentsInput =
  | Record<string, Component>
  | Map<string, Component>
  | InspectorPropertyDrawComponentRegistration[]
  | InspectorPropertyDrawLibrary
  | null
  | undefined;

export interface InspectorPropertyDrawOptions {
  component?: Component | null;
  components?: InspectorPropertyDrawComponentsInput;
  disabled?: boolean;
  readonly?: boolean;
  compact?: boolean;
  showLabel?: boolean;
  onCommit?: (commit: InspectorPropertyCommit) => void;
}

export interface InspectorPropertyDrawComponentProps {
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
  editable: boolean;
  disabled: boolean;
  readonly: boolean;
  compact: boolean;
  treeId: string;
  targetId: string;
  depth: number;
  children: InspectorProperty[];
  commit: (value: unknown, property?: InspectorProperty) => void;
  draw: (property?: InspectorProperty, options?: InspectorPropertyDrawOptions) => VNodeChild;
  drawChild: (propertyOrPath: InspectorProperty | string, options?: InspectorPropertyDrawOptions) => VNodeChild;
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
  drawComponents?: InspectorPropertyDrawComponentsInput;
  drawComponentResolvers?: InspectorPropertyDrawComponentResolver[];
  stateUpdaters?: InspectorStateUpdater[];
}

export type InspectorDrawerResolver = (
  property: InspectorProperty,
  context: InspectorPropertyTreeContext,
) => InspectorPropertyDrawer | null | undefined;

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
  drawComponents: NormalizedDrawComponentRegistry;
  drawComponentResolvers: InspectorPropertyDrawComponentResolver[];
  stateUpdaters: InspectorStateUpdater[];
}

interface InspectorPropertyInit {
  snapshot: InspectorPropertySnapshot;
  parent: InspectorProperty | null;
  depth: number;
  index: number;
}

interface InspectorPropertyDrawConfig {
  context: InspectorPropertyTreeContext;
  components: NormalizedDrawComponentRegistry;
  componentResolvers: InspectorPropertyDrawComponentResolver[];
}

interface NormalizedDrawComponentRegistration {
  types: string[];
  component: Component;
  match?: InspectorPropertyDrawMatcher;
  priority: number;
  order: number;
}

interface NormalizedDrawComponentRegistry {
  entries: NormalizedDrawComponentRegistration[];
  libraries: InspectorPropertyDrawLibrary[];
}

const VECTOR_TYPES = new Set(["Vector2", "Vector3", "Vector4", "Rect"]);
const NUMBER_TYPES = new Set(["Integer", "ArraySize", "Float"]);
const DEFAULT_AUTO_COLLAPSE_CHILD_COUNT = 24;
const EMPTY_DRAW_COMPONENT_REGISTRY: NormalizedDrawComponentRegistry = {
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
const DEFAULT_DRAW_CONFIG: InspectorPropertyDrawConfig = {
  context: DEFAULT_DRAW_CONTEXT,
  components: EMPTY_DRAW_COMPONENT_REGISTRY,
  componentResolvers: [],
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
  children: InspectorProperty[] = [];
  drawer: InspectorPropertyDrawer = {
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
  private drawConfig: InspectorPropertyDrawConfig = DEFAULT_DRAW_CONFIG;

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

  draw(options: InspectorPropertyDrawOptions = {}): VNodeChild {
    return drawInspectorProperty(this, options, this.drawConfig);
  }

  drawComponent(options: Pick<InspectorPropertyDrawOptions, "component" | "components"> = {}): Component | null {
    return resolveInspectorPropertyDrawComponent(this, options, this.drawConfig);
  }

  hasDrawComponent(options: Pick<InspectorPropertyDrawOptions, "component" | "components"> = {}): boolean {
    return this.drawComponent(options) !== null;
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

  _setDrawConfig(config: InspectorPropertyDrawConfig) {
    this.drawConfig = config;
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

  draw(options: InspectorPropertyDrawOptions = {}): VNodeChild {
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

export function resolveInspectorDrawer(property: InspectorProperty): InspectorPropertyDrawer {
  if (property.isArray) return drawer("array", "command", property.valueType, true);
  if (property.isManagedReference) return drawer("managedReference", "command", property.valueType, true);
  if (property.children.length > 0 || property.snapshot.hasChildren) {
    return drawer("object", "none", property.valueType, true);
  }

  const valueType = property.valueType || property.type;
  if (valueType === "Boolean") return drawer("boolean", "change", valueType);
  if (valueType === "Enum") return drawer(property.isFlagsEnum ? "flags" : "enum", "change", valueType);
  if (valueType === "LayerMask") return drawer("layerMask", "blur", valueType);
  if (NUMBER_TYPES.has(valueType)) return drawer("number", "blur", valueType);
  if (VECTOR_TYPES.has(valueType)) return drawer("vector", "blur", valueType);
  if (valueType === "Color") return drawer("color", "change", valueType);
  if (valueType === "ObjectReference") return drawer("objectReference", "blur", valueType);
  if (valueType === "String") return drawer("text", "blur", valueType);
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

export function defineInspectorPropertyDrawComponents(
  input: InspectorPropertyDrawComponentsInput,
): InspectorPropertyDrawComponentRegistration[] {
  return expandDrawComponentRegistrations(input).map((entry) => ({
    ...entry,
    type: normalizeDrawTypes(entry.type),
  }));
}

export function createInspectorPropertyDrawLibrary(
  input?: InspectorPropertyDrawComponentsInput,
): InspectorPropertyDrawLibrary {
  const library = new MutableInspectorPropertyDrawLibrary();
  for (const registration of expandDrawComponentRegistrations(input)) {
    library.register(registration);
  }
  return library;
}

class MutableInspectorPropertyDrawLibrary implements InspectorPropertyDrawLibrary {
  private readonly registeredComponents: InspectorPropertyDrawComponentRegistration[] = [];

  get registrations(): readonly InspectorPropertyDrawComponentRegistration[] {
    return this.registeredComponents;
  }

  register(registration: InspectorPropertyDrawComponentRegistration): () => void;
  register(
    type: string | string[],
    component: Component,
    options?: Omit<InspectorPropertyDrawComponentRegistration, "type" | "component">,
  ): () => void;
  register(
    registrationOrType: InspectorPropertyDrawComponentRegistration | string | string[],
    component?: Component,
    options: Omit<InspectorPropertyDrawComponentRegistration, "type" | "component"> = {},
  ): () => void {
    if (typeof registrationOrType === "string" || Array.isArray(registrationOrType)) {
      if (!component) return () => undefined;
      return this.register({
        ...options,
        type: registrationOrType,
        component,
      });
    }
    const registration = registrationOrType;
    if (!registration.component) return () => undefined;
    this.registeredComponents.push(registration);
    return () => {
      const index = this.registeredComponents.indexOf(registration);
      if (index >= 0) this.registeredComponents.splice(index, 1);
    };
  }

  clear() {
    this.registeredComponents.splice(0, this.registeredComponents.length);
  }

  resolve(property: InspectorProperty, context: InspectorPropertyTreeContext): Component | null {
    return findDrawComponent(
      property,
      context,
      normalizeDrawComponents(this.registeredComponents),
    );
  }
}

export const publicInspectorPropertyDrawLibrary = createInspectorPropertyDrawLibrary();
export const projectInspectorPropertyDrawLibrary = publicInspectorPropertyDrawLibrary;

export function normalizeInspectorPropertyDrawComponents(
  input: InspectorPropertyDrawComponentsInput,
): InspectorPropertyDrawComponentRegistration[] {
  return defineInspectorPropertyDrawComponents(input);
}

export function registerInspectorPropertyDrawComponent(
  type: string | string[],
  component: Component,
  options: Omit<InspectorPropertyDrawComponentRegistration, "type" | "component"> = {},
): () => void {
  return publicInspectorPropertyDrawLibrary.register(type, component, options);
}

export const propertyTreeService = {
  createTree: createPropertyTree,
  resolveDrawer: resolveInspectorDrawer,
  resolveManagedReferenceTypeOption,
  searchManagedReferenceTypeOptions,
  defineDrawComponents: defineInspectorPropertyDrawComponents,
  createDrawLibrary: createInspectorPropertyDrawLibrary,
  publicDrawLibrary: publicInspectorPropertyDrawLibrary,
  projectDrawLibrary: projectInspectorPropertyDrawLibrary,
  normalizeDrawComponents: normalizeInspectorPropertyDrawComponents,
  registerDrawComponent: registerInspectorPropertyDrawComponent,
};

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
  const drawConfig: InspectorPropertyDrawConfig = {
    context,
    components: options.drawComponents,
    componentResolvers: options.drawComponentResolvers,
  };
  for (const property of properties) {
    property._setDrawConfig(drawConfig);
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
): InspectorPropertyDrawer {
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
    drawComponents: normalizeDrawComponents(options.drawComponents),
    drawComponentResolvers: options.drawComponentResolvers ?? [],
    stateUpdaters: options.stateUpdaters ?? [],
  };
}

function drawInspectorProperty(
  property: InspectorProperty,
  options: InspectorPropertyDrawOptions,
  config: InspectorPropertyDrawConfig,
): VNodeChild {
  const component = resolveInspectorPropertyDrawComponent(property, options, config);
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
  const draw = (targetProperty = property, nextOptions: InspectorPropertyDrawOptions = {}) =>
    targetProperty.draw({ ...drawOptions, ...nextOptions });
  const drawChild = (
    propertyOrPath: InspectorProperty | string,
    nextOptions: InspectorPropertyDrawOptions = {},
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

function resolveInspectorPropertyDrawComponent(
  property: InspectorProperty,
  options: Pick<InspectorPropertyDrawOptions, "component" | "components">,
  config: InspectorPropertyDrawConfig,
): Component | null {
  if (options.component) return options.component;
  const optionComponent = findDrawComponent(
    property,
    config.context,
    normalizeDrawComponents(options.components),
  );
  if (optionComponent) return optionComponent;

  for (const resolver of config.componentResolvers) {
    const resolved = resolver(property, config.context);
    if (resolved) return resolved;
  }

  return findDrawComponent(property, config.context, config.components)
    ?? findDrawComponent(
      property,
      config.context,
      normalizeDrawComponents(publicInspectorPropertyDrawLibrary),
    );
}

function normalizeDrawComponents(
  input: InspectorPropertyDrawComponentsInput,
): NormalizedDrawComponentRegistry {
  if (!input) return EMPTY_DRAW_COMPONENT_REGISTRY;
  if (isInspectorPropertyDrawLibrary(input)) {
    return {
      entries: [],
      libraries: [input],
    };
  }
  const entries: NormalizedDrawComponentRegistration[] = [];
  let order = 0;
  const pushRegistration = (registration: InspectorPropertyDrawComponentRegistration) => {
    if (!registration.component) return;
    const types = normalizeDrawTypes(registration.type);
    if (!types.length && !registration.match) return;
    entries.push({
      types,
      component: registration.component,
      match: registration.match,
      priority: Number.isFinite(registration.priority) ? Number(registration.priority) : 0,
      order,
    });
    order += 1;
  };

  for (const registration of expandDrawComponentRegistrations(input)) {
    pushRegistration(registration);
  }

  entries.sort((left, right) => right.priority - left.priority || left.order - right.order);
  return entries.length ? { entries, libraries: [] } : EMPTY_DRAW_COMPONENT_REGISTRY;
}

function expandDrawComponentRegistrations(
  input: InspectorPropertyDrawComponentsInput,
): InspectorPropertyDrawComponentRegistration[] {
  if (!input) return [];
  if (isInspectorPropertyDrawLibrary(input)) return [...input.registrations];
  if (Array.isArray(input)) return [...input];
  if (input instanceof Map) {
    return Array.from(input.entries()).map(([type, component]) => ({ type, component }));
  }
  return Object.entries(input).map(([type, component]) => ({ type, component }));
}

function isInspectorPropertyDrawLibrary(value: unknown): value is InspectorPropertyDrawLibrary {
  return Boolean(
    value &&
    typeof value === "object" &&
    "registrations" in value &&
    "register" in value &&
    "resolve" in value,
  );
}

function normalizeDrawTypes(type: string | string[] | undefined): string[] {
  return (Array.isArray(type) ? type : [type])
    .map((value) => normalizeDrawTypeKey(value || ""))
    .filter(Boolean);
}

function findDrawComponent(
  property: InspectorProperty,
  context: InspectorPropertyTreeContext,
  registry: NormalizedDrawComponentRegistry,
): Component | null {
  if (!registry.entries.length && !registry.libraries.length) return null;
  const keys = drawLookupKeys(property);
  for (const entry of registry.entries) {
    if (entry.match?.(property, context)) return entry.component;
    if (entry.types.some((type) => type === "*" || keys.has(type))) return entry.component;
  }
  for (const library of registry.libraries) {
    const resolved = library.resolve(property, context);
    if (resolved) return resolved;
  }
  return null;
}

function drawLookupKeys(property: InspectorProperty): Set<string> {
  return new Set([
    property.valueType,
    property.type,
    property.fieldTypeFullName,
    property.fieldTypeAssembly,
    property.drawer.kind,
    property.managedReferenceFullTypename,
    property.managedReferenceFieldTypename,
    property.managedReferenceDisplayName,
  ]
    .map(normalizeDrawTypeKey)
    .filter(Boolean));
}

function normalizeDrawTypeKey(value: string): string {
  return value.trim().toLowerCase();
}

function drawInspectorPropertyFallback(
  property: InspectorProperty,
  options: InspectorPropertyDrawOptions,
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

function childDrawOptions(options: InspectorPropertyDrawOptions): InspectorPropertyDrawOptions {
  return {
    components: options.components,
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
): InspectorPropertyDrawer {
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
