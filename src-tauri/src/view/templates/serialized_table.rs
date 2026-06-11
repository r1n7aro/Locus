pub(super) fn table_config_ts() -> String {
    r#"import type {
  SerializedTableColumnConfig,
  SerializedTableSourceConfig,
  SerializedTableSourceProvider,
} from "@locus/components";

export type {
  SerializedTableColumnConfig,
  SerializedTableSourceConfig,
  SerializedTableSourceProvider,
} from "@locus/components";

export interface SerializedTableOptions {
  maxRows?: number;
}

export const tableColumns: SerializedTableColumnConfig[] = [
  { id: "name", label: "Name", propertyPath: "m_Name" },
];

export const tableSources: SerializedTableSourceConfig[] = [];

export const tableOptions: SerializedTableOptions = {
  maxRows: 1000,
};

export const tableSourceProviders: SerializedTableSourceProvider[] = [
  // {
  //   id: "entity-prefabs",
  //   label: "Entity Prefabs",
  //   resolve: async ({ searchAssets, fromAssetResults }) =>
  //     fromAssetResults(await searchAssets("t:prefab component:Entity", ["Assets"], 1000), {
  //       sourceKind: "component",
  //       componentType: "Entity",
  //     }),
  // },
  // {
  //   id: "idata-assets",
  //   label: "IData Assets",
  //   resolve: async ({ searchAssets, fromAssetResults }) =>
  //     fromAssetResults(await searchAssets("t:scriptableObject inherits:IData", ["Assets"], 1000), {
  //       sourceKind: "asset",
  //     }),
  // },
];
"#
    .to_string()
}

pub(super) fn app_vue(_name: &str) -> String {
    r##"<script setup lang="ts">
import { onMounted, ref } from "vue";
import { resolveSerializedTableSources, view } from "@locus/view-runtime";
import { SerializedTableView } from "@locus/components";
import type {
  SerializedTableCell,
  SerializedTableColumnConfig,
  SerializedTableCommitEvent,
  SerializedTableProgress,
  SerializedTableRow,
  SerializedTableSourceConfig,
} from "@locus/components";
import {
  tableColumns,
  tableOptions,
  tableSources,
  tableSourceProviders,
} from "./tableConfig";

interface SerializedTableResponse {
  ok: boolean;
  message: string;
  rows: SerializedTableRow[];
}

interface SerializedTableWriteResponse {
  ok: boolean;
  message: string;
  cell: SerializedTableCell;
}

const storageKey = "serialized-table.config";
const readMaxRows = Math.max(1, Math.min(tableOptions?.maxRows ?? 1000, 5000));

const columns = ref<SerializedTableColumnConfig[]>([...tableColumns]);
const rows = ref<SerializedTableRow[]>([]);
const resolvedSources = ref<SerializedTableSourceConfig[]>([]);
const loading = ref(false);
const statusText = ref("Loading table");
const errorText = ref("");
const savingCellKey = ref("");
const progress = ref<SerializedTableProgress | null>(null);
const columnWidths = ref<Record<string, number>>({});

function setProgress(title: string, info: string, value: number) {
  progress.value = { active: true, title, info, progress: value };
  statusText.value = [title, info].filter((part) => part.trim()).join(" · ");
}

async function loadStoredWidths() {
  try {
    const stored = await view.storage.get(storageKey) as { columnWidths?: Record<string, number> } | null;
    if (stored && typeof stored === "object" && stored.columnWidths) {
      columnWidths.value = stored.columnWidths;
    }
  } catch (error) {
    console.error("[serialized-table] Stored config read failed", error);
  }
}

function persistColumnWidths(widths: Record<string, number>) {
  columnWidths.value = widths;
  void view.storage.set(storageKey, { columnWidths: widths }).catch((error) => {
    console.error("[serialized-table] Stored config write failed", error);
  });
}

async function refresh() {
  loading.value = true;
  errorText.value = "";
  try {
    setProgress("Sources", "Resolving table rows", 0.15);
    const resolved = await resolveSerializedTableSources({
      providers: tableSourceProviders,
      sources: tableSources,
      searchAssets: (query, roots, limit) => view.assets.search(query, roots, limit),
      onProgress: (label, index, total) =>
        setProgress("Sources", `${label} (${index + 1}/${total})`, 0.15 + ((index + 1) / Math.max(1, total)) * 0.25),
    });
    resolvedSources.value = resolved.sources;
    if (resolved.errors.length) errorText.value = resolved.errors.join("\n");
    if (!resolved.sources.length || !columns.value.length) {
      rows.value = [];
      statusText.value = "No configured sources or columns";
      return;
    }
    setProgress("Unity", `${resolved.sources.length} sources, ${columns.value.length} columns`, 0.55);
    const response = await view.callScript("SerializedTableApi", "Read", {
      sources: resolved.sources,
      columns: columns.value,
      maxRows: readMaxRows,
    }) as SerializedTableResponse;
    setProgress("Table", "Rendering rows", 0.9);
    rows.value = Array.isArray(response.rows) ? response.rows : [];
    statusText.value = response.message || "Ready";
  } catch (error) {
    errorText.value = error instanceof Error ? error.message : String(error);
    console.error("[serialized-table] Read failed", error);
    statusText.value = "Read failed";
  } finally {
    loading.value = false;
    progress.value = null;
  }
}

async function commitCell(event: SerializedTableCommitEvent) {
  const source = resolvedSources.value.find((item) => item.id === event.row.id) ?? null;
  const column = columns.value.find((item) => item.id === event.cell.columnId) ?? null;
  if (!source || !column || !event.cell.editable || savingCellKey.value) return;
  savingCellKey.value = `${event.row.id}:${event.cell.columnId}`;
  errorText.value = "";
  try {
    const result = await view.callScript("SerializedTableApi", "Write", {
      source,
      column,
      propertyPath: event.propertyPath,
      valueJson: JSON.stringify(event.value),
    }) as SerializedTableWriteResponse;
    if (result.cell) {
      const row = rows.value.find((item) => item.id === event.row.id);
      const cellIndex = row ? row.cells.findIndex((item) => item.columnId === event.cell.columnId) : -1;
      if (row && cellIndex >= 0) row.cells[cellIndex] = result.cell;
    }
    statusText.value = result.message || "Saved";
  } catch (error) {
    errorText.value = error instanceof Error ? error.message : String(error);
    console.error("[serialized-table] Write failed", error);
    statusText.value = "Write failed";
  } finally {
    savingCellKey.value = "";
  }
}

onMounted(() => {
  void loadStoredWidths();
  void refresh();
});
</script>

<template>
  <main class="view-shell serialized-table-view" data-locus-template="serialized-table">
    <header class="view-toolbar">
      <div class="toolbar-title">
        <span>Serialized Table</span>
      </div>
      <div class="toolbar-actions">
        <button type="button" :disabled="loading" @click="refresh">
          {{ loading ? "Reading" : "Refresh" }}
        </button>
      </div>
    </header>

    <SerializedTableView
      :columns="columns"
      :rows="rows"
      :loading="loading"
      :status="statusText"
      :error="errorText"
      :progress="progress"
      :saving-cell-key="savingCellKey"
      :source-count="resolvedSources.length"
      :column-widths="columnWidths"
      @update:column-widths="persistColumnWidths"
      @commit="commitCell"
    />
  </main>
</template>
"##
    .to_string()
}

pub(super) fn style_css() -> String {
    super::common::style_css(
        r#".serialized-table-view > .locus-serialized-table {
  flex: 1;
  min-height: 0;
}
"#,
    )
}

pub(super) fn view_api_cs() -> String {
    r##"using System;
using System.Collections.Generic;
using System.Globalization;
using UnityEditor;
using UnityEngine;

public static class SerializedTableApi
{
    [Serializable]
    public sealed class ReadArgs
    {
        public SerializedTableSourceConfig[] sources;
        public SerializedTableColumnConfig[] columns;
        public int maxRows;
    }

    [Serializable]
    public sealed class WriteArgs
    {
        public SerializedTableSourceConfig source;
        public SerializedTableColumnConfig column;
        public string propertyPath;
        public string valueJson;
    }

    [Serializable]
    public sealed class SerializedTableSourceConfig
    {
        public string id;
        public string label;
        public string assetPath;
        public string guid;
        public string sourceKind;
        public string componentType;
        public int componentIndex;
        public SerializedTablePropertyOverride[] propertyOverrides;
    }

    [Serializable]
    public sealed class SerializedTablePropertyOverride
    {
        public string columnId;
        public string propertyPath;
    }

    [Serializable]
    public sealed class SerializedTableColumnConfig
    {
        public string id;
        public string label;
        public string propertyPath;
    }

    private sealed class SerializedTableRow
    {
        public string id;
        public string label;
        public string assetPath;
        public string sourceKind;
        public string typeName;
        public string status;
        public string message;
        public SerializedCell[] cells;
    }

    private sealed class SerializedCell
    {
        public string columnId;
        public string label;
        public string propertyPath;
        public string name;
        public string type;
        public string valueType;
        public string fieldTypeFullName;
        public string fieldTypeAssembly;
        public string referenceTypeFullName;
        public string referenceTypeAssembly;
        public object value;
        public string displayValue;
        public bool editable;
        public bool hasChildren;
        public bool isArray;
        public int arraySize;
        public bool ok;
        public string message;
        public bool isFlagsEnum;
        public int enumValueIndex;
        public long enumValueFlag;
        public Locus.LocusBridge.SerializedEnumOption[] enumOptions;
        public Locus.LocusBridge.SerializedPropertySnapshot[] children;
        public bool isManagedReference;
        public string managedReferenceFullTypename;
        public string managedReferenceFieldTypename;
        public string managedReferenceDisplayName;
        public Locus.LocusBridge.SerializedManagedReferenceTypeOption[] managedReferenceTypes;
    }

    private sealed class SerializedTableResponse
    {
        public bool ok;
        public string message;
        public SerializedTableRow[] rows;
    }

    private sealed class SerializedTableWriteResponse
    {
        public bool ok;
        public string message;
        public SerializedCell cell;
    }

    private sealed class ResolvedSource
    {
        public UnityEngine.Object obj;
        public string assetPath;
        public string sourceKind;
        public string typeName;
    }

    public static object Read(ReadArgs args)
    {
        args = args ?? new ReadArgs();
        SerializedTableSourceConfig[] sources = args.sources ?? new SerializedTableSourceConfig[0];
        SerializedTableColumnConfig[] columns = args.columns ?? new SerializedTableColumnConfig[0];
        int maxRows = args.maxRows > 0 ? Math.Min(args.maxRows, 5000) : 1000;
        var rows = new List<SerializedTableRow>();

        for (int i = 0; i < sources.Length && rows.Count < maxRows; i++)
            rows.Add(ReadSourceRow(sources[i], columns));

        return new SerializedTableResponse
        {
            ok = true,
            message = rows.Count == 0 ? "No configured assets" : "Ready",
            rows = rows.ToArray()
        };
    }

    public static object Write(WriteArgs args)
    {
        if (args == null)
            throw new Exception("Write arguments are required");
        if (args.source == null)
            throw new Exception("Source row is required");
        if (args.column == null)
            throw new Exception("Column is required");

        ResolvedSource source = ResolveSource(args.source);
        string rootPropertyPath = ResolvePropertyPath(args.source, args.column);
        string propertyPath = !string.IsNullOrWhiteSpace(args.propertyPath)
            ? args.propertyPath
            : rootPropertyPath;
        var serialized = new SerializedObject(source.obj);
        serialized.Update();
        SerializedProperty prop = serialized.FindProperty(propertyPath);
        if (prop == null)
            throw new Exception("SerializedProperty not found: " + propertyPath);
        if (!Locus.LocusBridge.IsSerializedPropertyWritable(prop))
            throw new Exception("SerializedProperty is read only: " + propertyPath);

        Locus.LocusBridge.SetSerializedPropertyValue(prop, args.valueJson);
        ApplySerializedChanges(serialized, source.obj);

        SerializedProperty updated = serialized.FindProperty(rootPropertyPath);
        return new SerializedTableWriteResponse
        {
            ok = true,
            message = "Saved",
            cell = BuildCell(args.source, args.column, updated != null ? updated : prop)
        };
    }

    private static SerializedTableRow ReadSourceRow(
        SerializedTableSourceConfig sourceConfig,
        SerializedTableColumnConfig[] columns)
    {
        if (sourceConfig == null)
            return ErrorRow(null, columns, "Source row is empty");

        try
        {
            ResolvedSource source = ResolveSource(sourceConfig);
            var serialized = new SerializedObject(source.obj);
            serialized.Update();
            var cells = new SerializedCell[columns.Length];
            for (int i = 0; i < columns.Length; i++)
            {
                SerializedTableColumnConfig column = columns[i];
                string propertyPath = ResolvePropertyPath(sourceConfig, column);
                SerializedProperty prop = string.IsNullOrWhiteSpace(propertyPath)
                    ? null
                    : serialized.FindProperty(propertyPath);
                cells[i] = prop == null
                    ? ErrorCell(column, propertyPath, "SerializedProperty not found")
                    : BuildCell(sourceConfig, column, prop);
            }

            return new SerializedTableRow
            {
                id = SafeId(sourceConfig),
                label = SafeLabel(sourceConfig),
                assetPath = source.assetPath,
                sourceKind = source.sourceKind,
                typeName = source.typeName,
                status = "ok",
                message = "Ready",
                cells = cells
            };
        }
        catch (Exception ex)
        {
            return ErrorRow(sourceConfig, columns, ex.Message);
        }
    }

    private static SerializedTableRow ErrorRow(
        SerializedTableSourceConfig sourceConfig,
        SerializedTableColumnConfig[] columns,
        string message)
    {
        var cells = new SerializedCell[columns.Length];
        for (int i = 0; i < columns.Length; i++)
            cells[i] = ErrorCell(columns[i], ResolvePropertyPath(sourceConfig, columns[i]), message);

        return new SerializedTableRow
        {
            id = SafeId(sourceConfig),
            label = SafeLabel(sourceConfig),
            assetPath = sourceConfig != null ? sourceConfig.assetPath ?? "" : "",
            sourceKind = sourceConfig != null ? sourceConfig.sourceKind ?? "" : "",
            typeName = "",
            status = "error",
            message = message,
            cells = cells
        };
    }

    private static SerializedCell BuildCell(
        SerializedTableSourceConfig source,
        SerializedTableColumnConfig column,
        SerializedProperty prop)
    {
        Locus.LocusBridge.SerializedPropertySnapshot snapshot =
            Locus.LocusBridge.SnapshotSerializedProperty(prop, 3, 32);
        bool writable = Locus.LocusBridge.IsSerializedPropertyWritable(prop);
        return new SerializedCell
        {
            columnId = SafeColumnId(column),
            label = SafeColumnLabel(column),
            propertyPath = snapshot.propertyPath,
            name = snapshot.name,
            type = snapshot.type,
            valueType = snapshot.valueType,
            fieldTypeFullName = snapshot.fieldTypeFullName,
            fieldTypeAssembly = snapshot.fieldTypeAssembly,
            referenceTypeFullName = snapshot.referenceTypeFullName,
            referenceTypeAssembly = snapshot.referenceTypeAssembly,
            value = snapshot.value,
            displayValue = snapshot.displayValue,
            editable = writable,
            hasChildren = snapshot.hasChildren,
            isArray = snapshot.isArray,
            arraySize = snapshot.arraySize,
            ok = true,
            message = writable ? "Editable" : "Read only",
            isFlagsEnum = snapshot.isFlagsEnum,
            enumValueIndex = snapshot.enumValueIndex,
            enumValueFlag = snapshot.enumValueFlag,
            enumOptions = snapshot.enumOptions,
            children = snapshot.children,
            isManagedReference = snapshot.isManagedReference,
            managedReferenceFullTypename = snapshot.managedReferenceFullTypename,
            managedReferenceFieldTypename = snapshot.managedReferenceFieldTypename,
            managedReferenceDisplayName = snapshot.managedReferenceDisplayName,
            managedReferenceTypes = snapshot.managedReferenceTypes
        };
    }

    private static SerializedCell ErrorCell(
        SerializedTableColumnConfig column,
        string propertyPath,
        string message)
    {
        return new SerializedCell
        {
            columnId = SafeColumnId(column),
            label = SafeColumnLabel(column),
            propertyPath = propertyPath ?? "",
            name = "",
            type = "Error",
            valueType = "Error",
            fieldTypeFullName = "",
            fieldTypeAssembly = "",
            referenceTypeFullName = "",
            referenceTypeAssembly = "",
            value = null,
            displayValue = "",
            editable = false,
            hasChildren = false,
            isArray = false,
            arraySize = -1,
            ok = false,
            message = message,
            isFlagsEnum = false,
            enumValueIndex = -1,
            enumValueFlag = 0,
            enumOptions = new Locus.LocusBridge.SerializedEnumOption[0],
            children = new Locus.LocusBridge.SerializedPropertySnapshot[0],
            isManagedReference = false,
            managedReferenceFullTypename = "",
            managedReferenceFieldTypename = "",
            managedReferenceDisplayName = "",
            managedReferenceTypes = new Locus.LocusBridge.SerializedManagedReferenceTypeOption[0]
        };
    }

    private static ResolvedSource ResolveSource(SerializedTableSourceConfig source)
    {
        string assetPath = ResolveAssetPath(source);
        if (string.IsNullOrWhiteSpace(assetPath))
            throw new Exception("Asset path is required");

        UnityEngine.Object asset = AssetDatabase.LoadMainAssetAtPath(assetPath);
        if (asset == null)
            throw new Exception("Asset not found: " + assetPath);

        string kind = string.IsNullOrWhiteSpace(source.sourceKind) ? "asset" : source.sourceKind.Trim();
        UnityEngine.Object obj = asset;
        if (string.Equals(kind, "component", StringComparison.OrdinalIgnoreCase))
            obj = ResolveComponent(asset, source);

        Type type = obj.GetType();
        return new ResolvedSource
        {
            obj = obj,
            assetPath = assetPath,
            sourceKind = kind,
            typeName = type.FullName ?? type.Name
        };
    }

    private static UnityEngine.Object ResolveComponent(
        UnityEngine.Object asset,
        SerializedTableSourceConfig source)
    {
        GameObject go = asset as GameObject;
        if (go == null)
            throw new Exception("Component source requires a prefab or GameObject asset");

        Component[] components = go.GetComponents<Component>();
        string componentType = source.componentType ?? "";
        int targetIndex = source.componentIndex < 0 ? 0 : source.componentIndex;
        int matchIndex = 0;
        for (int i = 0; i < components.Length; i++)
        {
            Component component = components[i];
            if (component == null)
                continue;
            Type type = component.GetType();
            if (!Locus.LocusBridge.TypeMatches(type, componentType))
                continue;
            if (matchIndex == targetIndex)
                return component;
            matchIndex++;
        }
        throw new Exception("Component not found: " + componentType);
    }

    private static string ResolveAssetPath(SerializedTableSourceConfig source)
    {
        if (source == null)
            return "";
        if (!string.IsNullOrWhiteSpace(source.guid))
        {
            string guidPath = AssetDatabase.GUIDToAssetPath(source.guid.Trim());
            if (!string.IsNullOrWhiteSpace(guidPath))
                return guidPath;
        }
        return source.assetPath ?? "";
    }

    private static string ResolvePropertyPath(
        SerializedTableSourceConfig source,
        SerializedTableColumnConfig column)
    {
        if (column == null)
            return "";
        if (source != null && source.propertyOverrides != null)
        {
            for (int i = 0; i < source.propertyOverrides.Length; i++)
            {
                SerializedTablePropertyOverride item = source.propertyOverrides[i];
                if (item != null &&
                    string.Equals(item.columnId, column.id, StringComparison.Ordinal) &&
                    !string.IsNullOrWhiteSpace(item.propertyPath))
                    return item.propertyPath;
            }
        }
        return column.propertyPath ?? "";
    }

    private static string SafeId(SerializedTableSourceConfig source)
    {
        if (source == null || string.IsNullOrWhiteSpace(source.id))
            return "source";
        return source.id;
    }

    private static string SafeLabel(SerializedTableSourceConfig source)
    {
        if (source == null)
            return "Source";
        if (!string.IsNullOrWhiteSpace(source.label))
            return source.label;
        if (!string.IsNullOrWhiteSpace(source.assetPath))
            return System.IO.Path.GetFileName(source.assetPath);
        return SafeId(source);
    }

    private static string SafeColumnId(SerializedTableColumnConfig column)
    {
        if (column == null || string.IsNullOrWhiteSpace(column.id))
            return "property";
        return column.id;
    }

    private static string SafeColumnLabel(SerializedTableColumnConfig column)
    {
        if (column == null)
            return "Property";
        if (!string.IsNullOrWhiteSpace(column.label))
            return column.label;
        return column.propertyPath ?? SafeColumnId(column);
    }

    private static void MarkObjectDirty(UnityEngine.Object obj)
    {
        if (obj == null)
            return;
        EditorUtility.SetDirty(obj);
        AssetDatabase.SaveAssetIfDirty(obj);
    }

    private static bool ApplySerializedChanges(SerializedObject serialized, UnityEngine.Object obj)
    {
        int undoGroup = Undo.GetCurrentGroup();
        Undo.SetCurrentGroupName("Locus Serialized Table");
        bool changed = serialized.ApplyModifiedProperties();
        if (changed)
        {
            RecordPrefabModifications(obj);
            MarkObjectDirty(obj);
            Undo.CollapseUndoOperations(undoGroup);
        }
        serialized.Update();
        return changed;
    }

    private static void RecordPrefabModifications(UnityEngine.Object obj)
    {
        if (obj == null)
            return;

        try
        {
            Component component = obj as Component;
            GameObject go = obj as GameObject;
            if (go == null && component != null)
                go = component.gameObject;
            if (go != null && PrefabUtility.GetNearestPrefabInstanceRoot(go) != null)
                PrefabUtility.RecordPrefabInstancePropertyModifications(obj);
        }
        catch
        {
        }
    }
}
"##
    .to_string()
}
