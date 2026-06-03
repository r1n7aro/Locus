import {
  BookText,
  Box,
  Clapperboard,
  File as LucideFile,
  FileBraces,
  FileCode,
  FileCog,
  FileImage,
  FileMusic,
  FileTerminal,
  FileText,
  FileType,
  FileVideo,
  Folder,
  FolderOpen,
  Map as MapIcon,
  Package as PackageIcon,
  Palette,
  Sparkles,
  type IconNode,
} from "lucide";

export type UnityAssetIconKind =
  | "scene"
  | "prefab"
  | "material"
  | "csharp"
  | "python"
  | "script"
  | "json"
  | "markdown"
  | "shader"
  | "texture"
  | "model"
  | "animation"
  | "audio"
  | "font"
  | "video"
  | "text"
  | "meta"
  | "gameobject"
  | "folder"
  | "asset"
  | "file";

export type UnityAssetIconTone = "primary" | "resource" | "media" | "neutral";

export const UNITY_FOLDER_OPEN_ICON = FolderOpen;

const FileHash: IconNode = [
  [
    "path",
    {
      d: "M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z",
    },
  ],
  ["path", { d: "M14 2v5a1 1 0 0 0 1 1h5" }],
  ["path", { d: "M10 12v6" }],
  ["path", { d: "M14 12v6" }],
  ["path", { d: "M8 14h8" }],
  ["path", { d: "M8 16h8" }],
];

export const UNITY_ASSET_ICON_NODES: Record<UnityAssetIconKind, IconNode> = {
  scene: MapIcon,
  prefab: Box,
  material: Palette,
  csharp: FileHash,
  python: FileTerminal,
  script: FileCode,
  json: FileBraces,
  markdown: BookText,
  shader: Sparkles,
  texture: FileImage,
  model: PackageIcon,
  animation: Clapperboard,
  audio: FileMusic,
  font: FileType,
  video: FileVideo,
  text: FileText,
  meta: FileCog,
  gameobject: Box,
  folder: Folder,
  asset: FileCog,
  file: LucideFile,
};

const UNITY_MATERIAL_EXTENSIONS = [".mat", ".physicmaterial", ".physicsmaterial2d"];
const UNITY_CSHARP_EXTENSIONS = [".cs"];
const UNITY_PYTHON_EXTENSIONS = [".py", ".pyw"];
const UNITY_SCRIPT_EXTENSIONS = [".asmdef", ".asmref", ".js", ".jsx", ".ts", ".tsx"];
const UNITY_JSON_EXTENSIONS = [".json", ".jsonc"];
const UNITY_MARKDOWN_EXTENSIONS = [".md", ".markdown", ".mdx"];
const UNITY_SHADER_EXTENSIONS = [".shader", ".shadergraph", ".compute", ".hlsl", ".cginc"];
const UNITY_TEXTURE_EXTENSIONS = [
  ".png",
  ".jpeg",
  ".jpg",
  ".tga",
  ".psd",
  ".bmp",
  ".gif",
  ".tiff",
  ".tif",
  ".exr",
  ".hdr",
  ".dds",
  ".svg",
  ".webp",
];
const UNITY_MODEL_EXTENSIONS = [".fbx", ".obj", ".blend", ".dae", ".3ds", ".ma", ".mb", ".max"];
const UNITY_ANIMATION_EXTENSIONS = [".anim", ".controller", ".overridecontroller", ".mask"];
const UNITY_AUDIO_EXTENSIONS = [".wav", ".mp3", ".ogg", ".aif", ".aiff", ".flac", ".xm", ".mod", ".it", ".s3m"];
const UNITY_FONT_EXTENSIONS = [".ttf", ".otf", ".fontsettings"];
const UNITY_VIDEO_EXTENSIONS = [".mp4", ".mov", ".webm", ".avi", ".mpeg", ".mpg"];
const UNITY_TEXT_EXTENSIONS = [".txt", ".xml", ".yaml", ".yml", ".csv", ".bytes", ".uxml", ".uss"];

export const UNITY_ASSET_ICON_FILE_EXTENSIONS = [
  ".unity",
  ".prefab",
  ".asset",
  ".meta",
  ...UNITY_MATERIAL_EXTENSIONS,
  ...UNITY_CSHARP_EXTENSIONS,
  ...UNITY_PYTHON_EXTENSIONS,
  ...UNITY_SCRIPT_EXTENSIONS,
  ...UNITY_JSON_EXTENSIONS,
  ...UNITY_MARKDOWN_EXTENSIONS,
  ...UNITY_SHADER_EXTENSIONS,
  ...UNITY_TEXTURE_EXTENSIONS,
  ...UNITY_MODEL_EXTENSIONS,
  ...UNITY_ANIMATION_EXTENSIONS,
  ...UNITY_AUDIO_EXTENSIONS,
  ...UNITY_FONT_EXTENSIONS,
  ...UNITY_VIDEO_EXTENSIONS,
  ...UNITY_TEXT_EXTENSIONS,
].sort((left, right) => right.length - left.length);

function hasExtension(fileName: string, extensions: readonly string[]) {
  return extensions.some((extension) => fileName.endsWith(extension));
}

interface IconKindOptions {
  isFolder?: boolean;
  isSceneObject?: boolean;
  fallbackKind?: Extract<UnityAssetIconKind, "asset" | "file">;
}

export function unityAssetIconKindForPath(filePath: string, options: IconKindOptions = {}): UnityAssetIconKind {
  const normalized = filePath.trim().replace(/\\/g, "/").replace(/\/+$/, "");
  if (options.isSceneObject || /^(?:Assets|Packages)\/.+?\.unity\/.+/i.test(normalized)) return "gameobject";
  if (options.isFolder === true || filePath.endsWith("/")) return "folder";

  const fileName = (normalized.split("/").pop() || normalized || filePath).toLowerCase();
  if (!fileName.includes(".")) return options.isFolder === false ? options.fallbackKind ?? "file" : "folder";
  if (fileName.endsWith(".unity")) return "scene";
  if (fileName.endsWith(".prefab")) return "prefab";
  if (hasExtension(fileName, UNITY_MATERIAL_EXTENSIONS)) return "material";
  if (hasExtension(fileName, UNITY_CSHARP_EXTENSIONS)) return "csharp";
  if (hasExtension(fileName, UNITY_PYTHON_EXTENSIONS)) return "python";
  if (hasExtension(fileName, UNITY_SCRIPT_EXTENSIONS)) return "script";
  if (hasExtension(fileName, UNITY_JSON_EXTENSIONS)) return "json";
  if (hasExtension(fileName, UNITY_MARKDOWN_EXTENSIONS)) return "markdown";
  if (hasExtension(fileName, UNITY_SHADER_EXTENSIONS)) return "shader";
  if (hasExtension(fileName, UNITY_TEXTURE_EXTENSIONS)) return "texture";
  if (hasExtension(fileName, UNITY_MODEL_EXTENSIONS)) return "model";
  if (hasExtension(fileName, UNITY_ANIMATION_EXTENSIONS)) return "animation";
  if (hasExtension(fileName, UNITY_AUDIO_EXTENSIONS)) return "audio";
  if (hasExtension(fileName, UNITY_FONT_EXTENSIONS)) return "font";
  if (hasExtension(fileName, UNITY_VIDEO_EXTENSIONS)) return "video";
  if (hasExtension(fileName, UNITY_TEXT_EXTENSIONS)) return "text";
  if (fileName.endsWith(".meta")) return "meta";
  if (fileName.endsWith(".asset")) return "asset";
  return options.fallbackKind ?? "file";
}

export function unityAssetIconNodeForKind(kind: UnityAssetIconKind): IconNode {
  return UNITY_ASSET_ICON_NODES[kind] ?? UNITY_ASSET_ICON_NODES.file;
}

export function unityAssetIconNodeForPath(filePath: string, options: IconKindOptions = {}): IconNode {
  return unityAssetIconNodeForKind(unityAssetIconKindForPath(filePath, options));
}

export function unityFolderIconNode(open: boolean): IconNode {
  return open ? UNITY_FOLDER_OPEN_ICON : UNITY_ASSET_ICON_NODES.folder;
}

export function unityAssetIconToneForKind(kind: UnityAssetIconKind): UnityAssetIconTone {
  if (kind === "scene" || kind === "prefab" || kind === "model" || kind === "gameobject") {
    return "primary";
  }
  if (kind === "material" || kind === "shader" || kind === "texture") {
    return "resource";
  }
  if (kind === "animation" || kind === "audio" || kind === "video") {
    return "media";
  }
  return "neutral";
}

export function unityAssetIconClassForKind(kind: UnityAssetIconKind): string {
  return [
    "unity-asset-icon",
    `unity-asset-icon--${kind}`,
    `unity-asset-icon--tone-${unityAssetIconToneForKind(kind)}`,
  ].join(" ");
}

export function unityAssetIconClassForPath(filePath: string, options: IconKindOptions = {}): string {
  return unityAssetIconClassForKind(unityAssetIconKindForPath(filePath, options));
}

export function unityFolderIconClass(open: boolean): string {
  return [
    unityAssetIconClassForKind("folder"),
    open ? "unity-asset-icon--folder-open" : "unity-asset-icon--folder-solid",
  ].join(" ");
}
