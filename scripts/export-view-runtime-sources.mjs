import { copyFileSync, mkdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const outRoot = path.join(repoRoot, "skills", "view", "app", "view-runtime");

const sourceFiles = [
  "src/components/view/viewRuntime.ts",
  "src/components/view/viewCompiler.ts",
  "src/components/view/viewSfcCompiler.ts",
  "src/components/view/viewPackageFiles.ts",
  "src/components/view/viewHostPreview.ts",
  "src/components/canvas/index.ts",
  "src/components/canvas/canvasTypes.ts",
  "src/components/canvas/canvasStyles.ts",
  "src/components/canvas/LocusCanvasView.ts",
  "src/components/graph/index.ts",
  "src/components/graph/graphTypes.ts",
  "src/components/graph/graphController.ts",
  "src/components/graph/graphLayout.ts",
  "src/components/graph/graphStyles.ts",
  "src/components/graph/graphImage.ts",
  "src/components/graph/LocusGraphView.ts",
  "src/components/table/index.ts",
  "src/components/table/serializedTable.ts",
  "src/components/table/SerializedTableView.vue",
  "src/components/icons/LucideIcon.vue",
  "src/components/icons/unityAssetIcons.ts",
  "src/components/diff/BinaryPreviewHost.vue",
  "src/components/diff/FbxBinaryPreview.vue",
  "src/components/diff/RasterBinaryPreview.vue",
  "src/components/diff/UnityHierarchyPane.vue",
  "src/components/diff/UnityInspectorPane.vue",
  "src/components/diff/UnityInspectorFieldTree.vue",
  "src/components/diff/OptimizedPanelRenderer.vue",
  "src/components/diff/ParticleSystemPanelRenderer.vue",
  "src/components/diff/fieldUtils.ts",
  "src/components/diff/inspectorPanelDisplay.ts",
  "src/components/diff/particleSystemSemantic.ts",
  "src/components/diff/rasterPreview.ts",
  "src/components/diff/rendererRegistry.ts",
  "src/components/diff/unityInspectorFieldState.ts",
  "src/components/diff/unityInspectorHeader.ts",
  "src/components/collab/mergeUi.ts",
  "src/components/unity/index.ts",
  "src/components/unity/unitySerializedValue.ts",
  "src/components/unity/UnityBoolField.vue",
  "src/components/unity/UnityColorField.vue",
  "src/components/unity/UnityEnumField.vue",
  "src/components/unity/UnityFlagsField.vue",
  "src/components/unity/UnityLayerMaskField.vue",
  "src/components/unity/UnityNumberField.vue",
  "src/components/unity/UnityObjectReferenceField.vue",
  "src/components/unity/UnityPropertyDraw.vue",
  "src/components/unity/UnityPropertyEditor.vue",
  "src/components/unity/UnitySerializedPropertyTree.vue",
  "src/components/unity/UnityVectorField.vue",
  "src/components/unity/unityPropertyBinding.ts",
  "src/components/unity-preview/index.ts",
  "src/components/unity-preview/unityObjectPreview.ts",
  "src/components/unity-preview/UnityObjectEditorPanel.vue",
  "src/components/unity-preview/UnityObjectIdentity.vue",
  "src/components/unity-preview/UnityObjectPreview.vue",
  "src/components/ui/BaseButton.vue",
  "src/components/ui/BaseCheckbox.vue",
  "src/components/ui/BaseDropdown.vue",
  "src/components/ui/BaseSegmented.vue",
  "src/components/ui/BaseSwitch.vue",
  "src/components/ui/floatingPosition.ts",
  "src/composables/useUnityAssetDropTarget.ts",
  "src/composables/useUnityReferenceDragSource.ts",
  "src/composables/resizeObserver.ts",
  "src/composables/useSelectionLock.ts",
  "src/i18n.ts",
  "src/language/en.json",
  "src/language/zh.json",
  "src/services/asset.ts",
  "src/services/diff.ts",
  "src/services/refGraph.ts",
  "src/services/view.ts",
  "src/services/unity.ts",
  "src/services/unitySerializedProperty.ts",
  "src/services/unityObjectDrawer.ts",
  "src/services/unityObjectReferencePicker.ts",
  "src/services/propertyTree.ts",
  "src/services/unityPropertyPath.ts",
  "src/services/startupPerf.ts",
  "src/services/errors.ts",
  "src/services/ipc.ts",
  "src/services/locusRuntime.ts",
  "src/types.ts",
];

function assertInside(parent, child) {
  const relative = path.relative(parent, child);
  if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error(`Refusing to write outside ${parent}: ${child}`);
  }
}

function copySource(relativePath) {
  const sourcePath = path.join(repoRoot, relativePath);
  const stats = statSync(sourcePath);
  if (!stats.isFile()) {
    throw new Error(`View runtime export source is not a file: ${relativePath}`);
  }

  const targetPath = path.join(outRoot, relativePath);
  assertInside(outRoot, targetPath);
  mkdirSync(path.dirname(targetPath), { recursive: true });
  copyFileSync(sourcePath, targetPath);
  return {
    path: relativePath,
    bytes: stats.size,
  };
}

assertInside(path.join(repoRoot, "skills", "view"), outRoot);
rmSync(outRoot, { recursive: true, force: true });
mkdirSync(outRoot, { recursive: true });

const files = sourceFiles.map(copySource);
writeFileSync(
  path.join(outRoot, "manifest.json"),
  `${JSON.stringify({
    schema: "locus.view-runtime-source-export.v1",
    description: "Release-readable source export for Locus View Runtime APIs used by generated View packages.",
    sourceRoot: "src",
    files,
  }, null, 2)}\n`,
);

console.log(`[locus] Exported ${files.length} View Runtime source files to ${path.relative(repoRoot, outRoot)}`);
