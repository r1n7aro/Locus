import type { AssetSearchResult } from "../../types";

export type { AssetSearchResult };

export interface SerializedTableColumnConfig {
  id: string;
  label: string;
  propertyPath: string;
}

export interface SerializedTablePropertyOverride {
  columnId: string;
  propertyPath: string;
}

export interface SerializedTableSourceConfig {
  id: string;
  label: string;
  assetPath: string;
  guid?: string;
  sourceKind?: "asset" | "component";
  componentType?: string;
  componentIndex?: number;
  propertyOverrides?: SerializedTablePropertyOverride[];
}

export interface SerializedTableSourceProviderContext {
  searchAssets(query: string, roots?: string[], limit?: number): Promise<AssetSearchResult[]>;
  fromAssetResults(
    results: AssetSearchResult[],
    defaults?: Partial<SerializedTableSourceConfig>,
  ): SerializedTableSourceConfig[];
}

export type SerializedTableSourceProvider =
  | SerializedTableSourceConfig[]
  | ((context: SerializedTableSourceProviderContext) =>
      SerializedTableSourceConfig[] | Promise<SerializedTableSourceConfig[]>)
  | {
      id: string;
      label?: string;
      resolve: (context: SerializedTableSourceProviderContext) =>
        SerializedTableSourceConfig[] | Promise<SerializedTableSourceConfig[]>;
    };

export interface SerializedTableEnumOption {
  label: string;
  value: string;
  name: string;
  index: number;
  numericValue: number;
}

export interface SerializedTableManagedReferenceType {
  label: string;
  value: string;
  fullName: string;
  assembly: string;
}

export interface SerializedTableCell {
  columnId: string;
  label: string;
  propertyPath: string;
  name: string;
  type: string;
  valueType: string;
  fieldTypeFullName: string;
  fieldTypeAssembly: string;
  referenceTypeFullName: string;
  referenceTypeAssembly: string;
  value: unknown;
  displayValue: string;
  editable: boolean;
  hasChildren: boolean;
  isArray: boolean;
  arraySize: number;
  ok: boolean;
  message: string;
  isFlagsEnum: boolean;
  enumValueIndex: number;
  enumValueFlag: number;
  enumOptions: SerializedTableEnumOption[];
  children: SerializedTableCell[];
  isManagedReference: boolean;
  managedReferenceFullTypename: string;
  managedReferenceFieldTypename: string;
  managedReferenceDisplayName: string;
  managedReferenceTypes: SerializedTableManagedReferenceType[];
}

export interface SerializedTableRow {
  id: string;
  label: string;
  assetPath: string;
  sourceKind: string;
  typeName: string;
  status: string;
  message: string;
  cells: SerializedTableCell[];
}

export interface SerializedTableProgress {
  active: boolean;
  title: string;
  info: string;
  progress: number;
}

export interface SerializedTableCommitEvent {
  row: SerializedTableRow;
  cell: SerializedTableCell;
  propertyPath: string;
  value: unknown;
}

function normalizeSerializedTableId(value: string, fallback: string): string {
  const normalized = value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  return normalized || fallback;
}

function serializedTableAssetName(path: string): string {
  const name = path.split("/").pop() || path;
  return name.replace(/\.[^.]+$/, "");
}

export function normalizeSerializedTableSource(
  source: SerializedTableSourceConfig,
  fallbackId: string,
): SerializedTableSourceConfig {
  const assetPath = source.assetPath || "";
  return {
    ...source,
    id: source.id || normalizeSerializedTableId(assetPath || fallbackId, fallbackId),
    label: source.label || (assetPath ? serializedTableAssetName(assetPath) : fallbackId),
    sourceKind: source.sourceKind || "asset",
    componentIndex: source.componentIndex ?? 0,
  };
}

function serializedTableSourceKey(source: SerializedTableSourceConfig): string {
  return [
    source.id,
    source.guid || "",
    source.assetPath || "",
    source.sourceKind || "asset",
    source.componentType || "",
    String(source.componentIndex ?? 0),
  ].join("|");
}

export function dedupeSerializedTableSources(
  sources: SerializedTableSourceConfig[],
): SerializedTableSourceConfig[] {
  const seen = new Set<string>();
  const rows: SerializedTableSourceConfig[] = [];
  for (const source of sources) {
    const key = serializedTableSourceKey(source);
    if (seen.has(key)) continue;
    seen.add(key);
    rows.push(source);
  }
  return rows;
}

export function serializedTableSourcesFromAssets(
  results: AssetSearchResult[],
  defaults: Partial<SerializedTableSourceConfig> = {},
): SerializedTableSourceConfig[] {
  return results.map((result, index) => normalizeSerializedTableSource({
    ...defaults,
    id: defaults.id
      ? `${defaults.id}-${index + 1}`
      : normalizeSerializedTableId(result.path.replace(/\.[^.]+$/, ""), `asset-${index + 1}`),
    label: defaults.label || result.name.replace(/\.[^.]+$/, ""),
    assetPath: result.path,
    componentIndex: defaults.componentIndex ?? 0,
  }, `asset-${index + 1}`));
}

function serializedTableProviderLabel(provider: SerializedTableSourceProvider, index: number): string {
  if (Array.isArray(provider)) return `Provider ${index + 1}`;
  if (typeof provider === "function") return provider.name || `Provider ${index + 1}`;
  return provider.label || provider.id || `Provider ${index + 1}`;
}

function normalizeSerializedTableProviderRows(
  rows: SerializedTableSourceConfig[],
  provider: SerializedTableSourceProvider,
  index: number,
): SerializedTableSourceConfig[] {
  const providerId = Array.isArray(provider)
    ? `provider-${index + 1}`
    : typeof provider === "function"
      ? normalizeSerializedTableId(provider.name || "", `provider-${index + 1}`)
      : normalizeSerializedTableId(provider.id || provider.label || "", `provider-${index + 1}`);
  return rows.map((row, rowIndex) => {
    const normalized = normalizeSerializedTableSource(row, `${providerId}-${rowIndex + 1}`);
    return {
      ...normalized,
      id: normalized.id.startsWith(`${providerId}-`) ? normalized.id : `${providerId}-${normalized.id}`,
    };
  });
}

export interface SerializedTableResolveSourcesOptions {
  providers?: SerializedTableSourceProvider[];
  sources?: SerializedTableSourceConfig[];
  searchAssets: (query: string, roots?: string[], limit?: number) => Promise<AssetSearchResult[]>;
  onProgress?: (label: string, index: number, total: number) => void;
}

export interface SerializedTableResolvedSources {
  sources: SerializedTableSourceConfig[];
  errors: string[];
}

export async function resolveSerializedTableSources(
  options: SerializedTableResolveSourcesOptions,
): Promise<SerializedTableResolvedSources> {
  const providers = options.providers ?? [];
  const manual = (options.sources ?? []).map((source, index) =>
    normalizeSerializedTableSource(source, `row-${index + 1}`));
  const errors: string[] = [];
  const scripted: SerializedTableSourceConfig[] = [];
  const context: SerializedTableSourceProviderContext = {
    searchAssets: (query, roots = ["Assets", "Packages"], limit = 1000) =>
      options.searchAssets(query, roots, limit),
    fromAssetResults: serializedTableSourcesFromAssets,
  };

  for (let index = 0; index < providers.length; index += 1) {
    const provider = providers[index];
    const label = serializedTableProviderLabel(provider, index);
    options.onProgress?.(label, index, providers.length);
    try {
      const rows = Array.isArray(provider)
        ? provider
        : typeof provider === "function"
          ? await provider(context)
          : await provider.resolve(context);
      if (Array.isArray(rows)) {
        scripted.push(...normalizeSerializedTableProviderRows(rows, provider, index));
      }
    } catch (error) {
      console.error(`[serialized-table] Source provider failed: ${label}`, error);
      const message = error instanceof Error ? error.message : String(error);
      errors.push(`${label}: ${message}`);
    }
  }

  return {
    sources: dedupeSerializedTableSources([...manual, ...scripted]),
    errors,
  };
}
