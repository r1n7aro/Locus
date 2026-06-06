import {
  PanelTopOpen,
  View as ViewIcon,
  type IconNode,
} from "lucide";
import {
  UNITY_ASSET_ICON_FILE_EXTENSIONS,
  type UnityAssetIconKind,
  unityAssetIconClassForKind,
  unityAssetIconKindForPath,
  unityAssetIconNodeForKind,
  unityFolderIconClass,
} from "../components/icons/unityAssetIcons";

/**
 * Pure functions for injecting interactive elements (asset chips, file refs)
 * into rendered Markdown HTML. Extracted for testability.
 */

export type MarkdownUnityObjectPreviewLevel = "inline" | "row" | "thumbnail" | "inspector" | "editor";
export type MarkdownPathEntryKind = "file" | "folder";

export interface MarkdownPathStatus {
  path: string;
  exists: boolean;
  entryKind?: MarkdownPathEntryKind | null;
}

export interface MarkdownRefInjectOptions {
  inlinePathStatus?: (path: string) => MarkdownPathStatus | null | undefined;
  requireInlinePathStatus?: boolean;
}

interface MarkdownUnityObjectRefOptions {
  level?: MarkdownUnityObjectPreviewLevel;
  editable?: boolean;
}

/**
 * Walk HTML string, applying `transform` only to text segments outside
 * code/pre blocks and anchor tags. Tags and protected content pass through.
 */
export function walkHtmlText(html: string, transform: (text: string) => string): string {
  const parts = html.split(/(<[^>]+>)/);
  let inCode = 0;
  let inAnchor = 0;
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    if (part.startsWith("<")) {
      if (/^<(code|pre)[\s>]/i.test(part)) inCode++;
      else if (/^<\/(code|pre)>/i.test(part)) inCode = Math.max(0, inCode - 1);
      if (/^<a[\s>]/i.test(part)) inAnchor++;
      else if (/^<\/a>/i.test(part)) inAnchor = Math.max(0, inAnchor - 1);
      continue;
    }
    if (inCode > 0 || inAnchor > 0) continue;
    parts[i] = transform(part);
  }
  return parts.join("");
}

const ASSET_ROOT_RE = /^(?:Assets|Packages)\//;
const SCENE_OBJECT_ROOT_RE = /^(?:Assets|Packages)\/.+?\.unity\/.+/i;
const QUOTED_SCENE_OBJECT_REF_RE = /(["'])@?((?:Assets|Packages)\/(?:(?!\1).)*?\.unity\/(?:(?!\1).)*?)\s*\1/g;
const QUOTED_ASSET_REF_RE = /(["'])@?((?:Assets|Packages)\/(?:(?!\1).)+?)\s*\1/g;
const BRACED_UNITY_REF_RE = /\{@?((?:Assets|Packages)\/[^{}\r\n]+?)\}/g;
const PARENTHESIZED_UNITY_ASSET_REF_RE = /\(@?((?:Assets|Packages)\/[^()\r\n]+?\.[A-Za-z0-9][^()\r\n]*?(?:#fileID:-?\d+)?)\)/gi;
const ASSET_REF_RE = /@((?:Assets|Packages)\/[\w.\/-]*[\w.-])(?!\/)/g;
const INLINE_CODE_BRACED_REF_RE = /^\{@?([^{}\r\n]+\/[^{}\r\n]*)\}$/;
const INLINE_CODE_UNITY_REF_PREFIX_RE = /^(?:asset|unity|ref)(?::([A-Za-z-]+))?\s+(.+)$/i;
const INLINE_CODE_UNITY_REF_SUFFIX_RE = /^(.+?)\s+\|\s*([A-Za-z-]+)$/;
const INLINE_CODE_PATH_SUFFIX_RE = /^(.+?)(?::(\d+)|#L(\d+)|#fileID:-?\d+)?$/i;
const INLINE_CODE_SINGLE_SEGMENT_ROOT_RE = /^(?:Assets|Packages|ProjectSettings|Library|Temp|Logs|Locus)$/i;
const INLINE_WORKSPACE_ROOT_RE = /^(?:ProjectSettings|src|src-tauri|Library|Editor)\//i;
const INLINE_GENERIC_FILE_PATH_RE = /^(?:[^/\r\n]+\/)+[^/\r\n]+\.[A-Za-z0-9][^/\r\n]*$/;
const INLINE_SLASH_COMMAND_RE = /^\/[A-Za-z0-9_-]+(?:\s|$)/;
const UNQUOTED_SCENE_OBJECT_START_RE = /@(?:Assets|Packages)\//g;
const UNQUOTED_UNITY_ASSET_START_RE = /@(?:Assets|Packages)\//g;
const BARE_UNITY_ASSET_START_RE = /(?<![@`\/])(?:Assets|Packages)\//g;
const BRACED_WORKSPACE_MENTION_RE = /\{@([^{}\r\n]*\/[^{}\r\n]*)\}/g;
const WORKSPACE_MENTION_RE = /@((?:[^\s@<]+\/)+[^\s@<]*)/g;
const KNOWLEDGE_DOCUMENT_ROOT_RE = /^(design|memory|skill|reference)\/(.+\.md)$/i;
const KNOWLEDGE_DOCUMENT_FILE_RE = /^Locus\/knowledge\/(design|memory|skill|reference)\/(.+\.md)$/i;
const UNITY_ASSET_ICON_BASE = "/unity-asset-icons";
const WINDOWS_DRIVE_ABSOLUTE_RE = /^[A-Za-z]:[\\/]/;
const UNC_ABSOLUTE_RE = /^(?:\\\\|\/\/)[^\\/]+[\\/][^\\/]+/;
const POSIX_ABSOLUTE_RE = /^\/(?!\/)/;
const QUOTED_LOCAL_FILE_REF_RE = /(["'])((?:[A-Za-z]:[\\/]|\\\\|\/\/)(?:(?!\1).)+?)\s*\1/g;
// Bare `/...` is too ambiguous in prose such as `GameObject/Component`.
// POSIX absolute paths are still rendered when they appear inside inline code.
const ABSOLUTE_LOCAL_FILE_REF_RE = /(?<![@`\w])((?:[A-Za-z]:[\\/]\S*|\\\\[^\s\\/]+[\\/][^\s\\/]+(?:[\\/]\S*)?|\/\/[^\s/]+\/[^\s/]+(?:\/\S*)?))/g;
const TRAILING_FILE_REF_PUNCT_RE = /[.,;，。；、？！\])}）】》」』]+$/;
const VIEW_REF_VALUE_SOURCE = "([A-Za-z0-9][A-Za-z0-9._/-]{0,127}|\\{[^<>{}\\r\\n]{1,160}\\})";
const VIEW_REF_PARAGRAPH_RE = new RegExp(
  `<p>\\s*(?:<code>)?view:${VIEW_REF_VALUE_SOURCE}(?:</code>)?\\s*</p>`,
  "gi",
);
const CODE_BLOCK_RE = /<pre><code([^>]*)>([\s\S]*?)<\/code><\/pre>/gi;
const CODE_CLASS_LANGUAGE_RE = /\blanguage-([^"\s]+)/i;

function escapeAttr(source: string): string {
  return source
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function displayFileRef(filePath: string, line = ""): string {
  const displayPath = filePath.replace(/\/+$/, "") || filePath;
  const segments = displayPath.split("/");
  const fileName = segments[segments.length - 1] || displayPath;
  return line ? `${fileName}:${line}` : fileName;
}

function normalizeFileRefPath(filePath: string): string {
  return filePath.trim().replace(/\\/g, "/");
}

export function normalizeMarkdownPathStatusKey(filePath: string): string {
  const normalized = normalizeFileRefPath(filePath).replace(/^\.\//, "");
  return normalized.replace(/\/+$/, "") || normalized;
}

interface KnowledgeRefParts {
  docType: string;
  path: string;
}

function parseKnowledgeRefPath(filePath: string): KnowledgeRefParts | null {
  const normalized = normalizeFileRefPath(filePath).replace(/^\/+|\/+$/g, "");
  if (!normalized) return null;

  const fileMatch = normalized.match(KNOWLEDGE_DOCUMENT_FILE_RE);
  if (fileMatch) {
    const docType = fileMatch[1].toLowerCase();
    const path = normalizeFileRefPath(fileMatch[2]).replace(/^\/+|\/+$/g, "");
    return path ? { docType, path: `${docType}/${path}` } : null;
  }

  const rootMatch = normalized.match(KNOWLEDGE_DOCUMENT_ROOT_RE);
  if (!rootMatch) return null;
  const docType = rootMatch[1].toLowerCase();
  const path = normalizeFileRefPath(rootMatch[2]).replace(/^\/+|\/+$/g, "");
  return path ? { docType, path: `${docType}/${path}` } : null;
}

export function isAbsoluteLocalRefPath(filePath: string): boolean {
  const normalized = filePath.trim();
  return WINDOWS_DRIVE_ABSOLUTE_RE.test(normalized)
    || UNC_ABSOLUTE_RE.test(normalized)
    || POSIX_ABSOLUTE_RE.test(normalized);
}

function isUsableAbsoluteLocalRefPath(filePath: string): boolean {
  const normalized = normalizeFileRefPath(filePath);
  if (!isAbsoluteLocalRefPath(normalized)) return false;
  if (WINDOWS_DRIVE_ABSOLUTE_RE.test(normalized)) return normalized.length > 3;
  if (UNC_ABSOLUTE_RE.test(normalized)) return normalized.split("/").filter(Boolean).length >= 2;
  return normalized.length > 1;
}

function fileRefBaseName(filePath: string): string {
  const normalized = normalizeFileRefPath(filePath).replace(/\/+$/, "");
  const parts = normalized.split("/").filter(Boolean);
  return parts[parts.length - 1] || normalized;
}

function hasFileExtension(filePath: string): boolean {
  return /\.[^./\\]+$/.test(fileRefBaseName(filePath));
}

function isFolderFileRef(filePath: string, line = ""): boolean {
  if (line) return false;
  const raw = filePath.trim();
  if (/[\\/]$/.test(raw)) return true;
  return isAbsoluteLocalRefPath(raw) && !hasFileExtension(raw);
}

function normalizeUnityAssetRefPath(filePath: string): string {
  const normalized = filePath
    .trim()
    .replace(/\\/g, "/")
    .replace(/#fileID:-?\d+$/i, "");
  return normalized.replace(/\/+$/, "") || normalized;
}

function isUnityAssetRootPath(filePath: string): boolean {
  return /^(?:Assets|Packages)\/?$/i.test(normalizeFileRefPath(filePath));
}

function displaySceneObjectRef(objectPath: string): string {
  const normalized = objectPath.replace(/\/+$/, "") || objectPath;
  const segments = normalized.split("/").filter(Boolean);
  return segments[segments.length - 1] || normalized;
}

function unityAssetKind(filePath: string, isFolder?: boolean): UnityAssetIconKind {
  return unityAssetIconKindForPath(filePath, { fallbackKind: "asset", isFolder });
}

function unityAssetIconImageKind(kind: UnityAssetIconKind): UnityAssetIconKind {
  switch (kind) {
    case "csharp":
    case "python":
      return "script";
    case "json":
    case "markdown":
      return "text";
    default:
      return kind;
  }
}

function unityAssetIconSrc(kind: UnityAssetIconKind): string {
  return `${UNITY_ASSET_ICON_BASE}/${unityAssetIconImageKind(kind)}.svg`;
}

function renderSvgAttrs(attrs: Record<string, string | number | undefined>): string {
  return Object.entries(attrs)
    .filter((entry): entry is [string, string | number] => entry[1] !== undefined)
    .map(([key, value]) => ` ${key}="${escapeAttr(String(value))}"`)
    .join("");
}

function renderIconNode(icon: IconNode, classes = ""): string {
  const className = ["md-ref-icon", "md-ref-icon-lucide", classes].filter(Boolean).join(" ");
  const children = icon
    .map(([tag, attrs]) => `<${tag}${renderSvgAttrs(attrs)} />`)
    .join("");
  return `<svg class="${className}" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">${children}</svg>`;
}

function renderLucideRefIcon(kind: UnityAssetIconKind, classes = ""): string {
  const iconNode = unityAssetIconNodeForKind(kind);
  const sharedClasses = kind === "folder" ? unityFolderIconClass(false) : unityAssetIconClassForKind(kind);
  return renderIconNode(iconNode, [sharedClasses, classes].filter(Boolean).join(" "));
}

function renderRefIcon(kind: UnityAssetIconKind = "file", classes = ""): string {
  const className = ["md-ref-icon", "md-ref-icon-image", classes].filter(Boolean).join(" ");
  const image = `<img class="${className}" src="${unityAssetIconSrc(kind)}" alt="" aria-hidden="true" draggable="false" loading="lazy">`;
  return `${image}${renderLucideRefIcon(kind, classes)}`;
}

function renderUnityAssetIcon(kind: UnityAssetIconKind): string {
  return renderRefIcon(kind, `md-unity-asset-icon md-unity-asset-icon--${kind}`);
}

function markdownUnityPreviewLevelFromToken(token: string | undefined): MarkdownUnityObjectPreviewLevel | null {
  const normalized = token?.trim().toLowerCase();
  if (!normalized) return null;
  if (normalized === "inline" || normalized === "chip") return "inline";
  if (normalized === "row" || normalized === "line" || normalized === "block" || normalized === "editable") return "row";
  if (normalized === "preview" || normalized === "thumbnail" || normalized === "thumb") return "thumbnail";
  if (normalized === "inspector" || normalized === "inspect") return "inspector";
  if (normalized === "editor" || normalized === "edit") return "editor";
  return null;
}

function markdownUnityRefOptionsFromToken(token: string | undefined): MarkdownUnityObjectRefOptions | null {
  const level = markdownUnityPreviewLevelFromToken(token);
  if (!level) return null;
  const normalized = token?.trim().toLowerCase();
  return {
    level,
    editable: normalized === "editable" || normalized === "editor" || normalized === "edit",
  };
}

export function isMarkdownUnityObjectFenceLanguage(language: string | undefined): boolean {
  if (!language) return false;
  const normalized = language.trim().toLowerCase();
  return /^(?:asset|unity|ref)(?::|-)?(?:inline|chip|row|line|block|preview|thumbnail|thumb|inspector|inspect|editor|edit|editable)?$/.test(normalized);
}

export function isMarkdownUnityPropertyFenceLanguage(language: string | undefined): boolean {
  if (!language) return false;
  const normalized = language.trim().toLowerCase();
  return /^(?:unity[_-]?property|unity:property|property:unity|unity-property-editor)$/.test(normalized);
}

function markdownUnityRefOptionsFromFenceLanguage(language: string | undefined): MarkdownUnityObjectRefOptions | null {
  if (!isMarkdownUnityObjectFenceLanguage(language)) return null;
  const normalized = language?.trim().toLowerCase() ?? "";
  const match = normalized.match(/^(?:asset|unity|ref)(?::|-)?(.+)?$/);
  const tokenOptions = markdownUnityRefOptionsFromToken(match?.[1]);
  return tokenOptions ?? { level: "row" };
}

function normalizeMarkdownUnityObjectOptions(options: MarkdownUnityObjectRefOptions = {}): Required<MarkdownUnityObjectRefOptions> {
  const level = options.editable && (!options.level || options.level === "inline")
    ? "row"
    : options.level ?? "inline";
  return {
    level,
    editable: !!options.editable,
  };
}

function unityObjectPreviewAttrs(
  refKind: "asset" | "sceneObject",
  levelOptions: MarkdownUnityObjectRefOptions = {},
): { classes: string; attrs: string } {
  const options = normalizeMarkdownUnityObjectOptions(levelOptions);
  const classes = `md-unity-object-ref md-unity-object-ref--${options.level}`;
  const editableAttr = options.editable ? ` data-md-unity-editable="true"` : "";
  return {
    classes,
    attrs: ` data-md-unity-object-preview="true" data-md-unity-level="${options.level}" data-md-unity-ref-kind="${refKind}"${editableAttr}`,
  };
}

function renderFileRef(
  filePath: string,
  line = "",
  classes = "",
  attrs = "",
  icon = renderRefIcon(),
): string {
  const escaped = escapeAttr(filePath);
  const lineAttr = line ? ` data-file-line="${escapeAttr(line)}"` : "";
  const label = `${escaped}${line ? ":" + escapeAttr(line) : ""}`;
  const className = ["md-file-ref", classes, "ui-select-text"].filter(Boolean).join(" ");
  return `<span class="${className}" data-file-path="${escaped}"${lineAttr}${attrs} title="${label}" aria-label="${label}">${icon}<span class="md-ref-label">${displayFileRef(filePath, line)}</span></span>`;
}

function renderLocalFileRef(
  filePath: string,
  line = "",
  entryKind?: MarkdownPathEntryKind | null,
): string {
  const normalizedPath = normalizeFileRefPath(filePath);
  const isDir = entryKind ? entryKind === "folder" : isFolderFileRef(filePath, line);
  const cleanPath = isDir ? (normalizedPath.replace(/\/+$/, "") || normalizedPath) : normalizedPath;
  const classes = isDir ? "md-folder-ref" : "";
  const resolvedEntryKind = isDir ? "folder" : "file";
  const icon = isDir
    ? renderRefIcon("folder", "md-workspace-ref-icon")
    : renderRefIcon();
  return renderFileRef(cleanPath, line, classes, ` data-entry-kind="${resolvedEntryKind}" draggable="true"`, icon);
}

function renderUnityAssetRef(
  filePath: string,
  line = "",
  options: MarkdownUnityObjectRefOptions = {},
  status?: MarkdownPathStatus | null,
): string {
  const normalizedPath = normalizeUnityAssetRefPath(filePath);
  const escaped = escapeAttr(normalizedPath);
  const inferredFolder = isFolderFileRef(filePath, line) || !hasFileExtension(normalizedPath);
  const entryKind = status?.entryKind ?? (inferredFolder ? "folder" : "file");
  const kind = unityAssetKind(normalizedPath, entryKind === "folder");
  const preview = unityObjectPreviewAttrs("asset", options);
  const refClasses = [
    "md-unity-asset-ref",
    entryKind === "folder" ? "md-folder-ref" : "",
    preview.classes,
  ].filter(Boolean).join(" ");
  return renderFileRef(
    normalizedPath,
    line,
    refClasses,
    `${preview.attrs} data-asset-path="${escaped}" data-asset-kind="${kind}" data-entry-kind="${entryKind}" draggable="true"`,
    renderUnityAssetIcon(kind),
  );
}

function renderKnowledgeRef(filePath: string): string {
  const ref = parseKnowledgeRefPath(filePath);
  if (!ref) return escapeAttr(filePath);
  const escapedPath = escapeAttr(ref.path);
  const escapedType = escapeAttr(ref.docType);
  const icon = renderLucideRefIcon("text", "md-knowledge-ref-icon");
  return `<span class="md-file-ref md-knowledge-ref ui-select-text" data-knowledge-type="${escapedType}" data-knowledge-path="${escapedPath}" data-entry-kind="knowledge" title="${escapedPath}" aria-label="${escapedPath}">${icon}<span class="md-ref-label">${displayFileRef(ref.path)}</span></span>`;
}

function renderInlineCommandRef(source: string): string {
  const display = source.trim();
  const command = display.match(/^\/[A-Za-z0-9_-]+/)?.[0] ?? display;
  const escapedDisplay = escapeAttr(display);
  const escapedCommand = escapeAttr(command);
  return `<code class="md-command-ref" data-command-trigger="${escapedCommand}" title="${escapedDisplay}" aria-label="${escapedDisplay}">${escapedDisplay}</code>`;
}

export interface ViewRefInjectOptions {
  openLabel?: string;
  viewLabel?: string;
  resolveViewRef?: (viewId: string) => {
    id?: string | null;
    icon?: IconNode | null;
    iconName?: string | null;
  } | null;
}

function normalizeViewRefValue(source: string): string {
  const decoded = decodeCodeText(source).trim();
  const unwrapped = decoded.startsWith("{") && decoded.endsWith("}")
    ? decoded.slice(1, -1).trim()
    : decoded;
  return unwrapped.replace(/\\/g, "/").replace(/\/+/g, "/").replace(/^\/+|\/+$/g, "");
}

function renderViewRefBlock(source: string, options: ViewRefInjectOptions = {}): string {
  const viewId = normalizeViewRefValue(source);
  if (!viewId || /[\r\n<>]/.test(viewId)) return escapeAttr(`view:${source}`);

  const resolved = options.resolveViewRef?.(viewId) ?? null;
  const resolvedViewId = normalizeViewRefValue(resolved?.id ?? "") || viewId;
  const openLabel = options.openLabel?.trim() || "Open View";
  const viewLabel = options.viewLabel?.trim() || "View";
  const escapedViewId = escapeAttr(viewId);
  const escapedRunViewId = escapeAttr(resolvedViewId);
  const escapedOpenLabel = escapeAttr(openLabel);
  const escapedViewLabel = escapeAttr(viewLabel);
  const title = `${escapedViewLabel}: ${escapedViewId}`;
  const viewIcon = renderIconNode(resolved?.icon ?? ViewIcon, "md-view-ref-kind-icon");
  const openIcon = renderIconNode(PanelTopOpen, "md-view-open-icon");
  const iconAttr = resolved?.iconName?.trim()
    ? ` data-view-icon="${escapeAttr(resolved.iconName.trim())}"`
    : "";
  const sourceAttr = resolvedViewId !== viewId
    ? ` data-view-ref="${escapedViewId}"`
    : "";

  return [
    `<div class="md-view-ref-block ui-select-text" data-view-id="${escapedRunViewId}"${sourceAttr}${iconAttr} title="${title}" aria-label="${title}">`,
    `<div class="md-view-ref-main">${viewIcon}<span class="md-view-ref-type">${escapedViewLabel}</span><span class="md-view-ref-id">${escapedViewId}</span></div>`,
    `<button type="button" class="md-view-open-button ui-select-none" data-view-id="${escapedRunViewId}" title="${escapedOpenLabel}" aria-label="${escapedOpenLabel}: ${escapedViewId}">${openIcon}<span>${escapedOpenLabel}</span></button>`,
    "</div>",
  ].join("");
}

export function injectViewRefs(html: string, options: ViewRefInjectOptions = {}): string {
  return html.replace(VIEW_REF_PARAGRAPH_RE, (_match, value) =>
    renderViewRefBlock(value, options),
  );
}

function renderWorkspaceMention(
  path: string,
  match: string,
  status?: MarkdownPathStatus | null,
): string {
  const isDir = status?.entryKind === "folder" || (!status && path.endsWith("/"));
  const knowledgeRef = parseKnowledgeRefPath(path);
  if (knowledgeRef) {
    return renderKnowledgeRef(knowledgeRef.path);
  }

  if (isUsableAbsoluteLocalRefPath(path)) {
    return renderLocalFileRef(path, "", status?.entryKind);
  }

  if (/^(Assets|Packages)\//.test(path) && !isDir) {
    return match;
  }

  const normalizedPath = path.replace(/\/+$/, "");
  if (!normalizedPath) {
    return match;
  }

  const escapedPath = escapeAttr(normalizedPath);
  const segments = normalizedPath.split("/").filter(Boolean);
  const name = segments[segments.length - 1] || normalizedPath;
  const title = `${escapedPath}${isDir ? "/" : ""}`;
  const fileAttr = isDir ? "" : ` data-file-path="${escapedPath}"`;
  const classes = isDir ? "md-workspace-ref md-folder-ref" : "md-workspace-ref md-file-ref";
  const icon = isDir
    ? renderRefIcon("folder", "md-workspace-ref-icon")
    : renderLucideRefIcon("file", "md-workspace-ref-icon md-workspace-file-icon");

  return `<span class="${classes} ui-select-text" data-workspace-path="${escapedPath}" data-entry-kind="${isDir ? "folder" : "file"}"${fileAttr} draggable="true" title="${title}" aria-label="${title}">${icon}<span class="md-workspace-ref-prefix">@</span>${escapeAttr(name)}${isDir ? "/" : ""}</span>`;
}

interface SceneObjectRefParts {
  scenePath: string;
  objectPath: string;
}

function splitSceneObjectRef(filePath: string): SceneObjectRefParts | null {
  const normalized = filePath.trim().replace(/\\/g, "/").replace(/\/+$/, "");
  const match = normalized.match(/^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i);
  if (!match) return null;
  const scenePath = match[1];
  const objectPath = match[2].replace(/^\/+|\/+$/g, "");
  if (!scenePath || !objectPath) return null;
  return { scenePath, objectPath };
}

function renderUnitySceneObjectRef(
  filePath: string,
  options: MarkdownUnityObjectRefOptions = {},
): string {
  const ref = splitSceneObjectRef(filePath);
  if (!ref) return escapeAttr(filePath);
  const fullPath = `${ref.scenePath}/${ref.objectPath}`;
  const escapedFullPath = escapeAttr(fullPath);
  const escapedScenePath = escapeAttr(ref.scenePath);
  const escapedObjectPath = escapeAttr(ref.objectPath);
  const escapedLabel = escapeAttr(displaySceneObjectRef(ref.objectPath));
  const icon = renderRefIcon("gameobject", "md-unity-gameobject-icon");
  const preview = unityObjectPreviewAttrs("sceneObject", options);
  return `<span class="md-file-ref md-unity-scene-object-ref ${preview.classes} ui-select-text"${preview.attrs} data-file-path="${escapedFullPath}" data-scene-path="${escapedScenePath}" data-scene-object-path="${escapedObjectPath}" draggable="true" title="${escapedFullPath}" aria-label="${escapedFullPath}">${icon}<span class="md-ref-label">${escapedLabel}</span></span>`;
}

function isSceneObjectRefTerminator(ch: string): boolean {
  return /[\r\n<>"'`{}，。；、？！]/.test(ch);
}

export function findUnitySceneObjectPathEnd(text: string, start: number): number {
  const lower = text.toLowerCase();
  const sceneMarker = lower.indexOf(".unity/", start);
  if (sceneMarker < 0 || text.slice(start, sceneMarker).includes("@")) {
    return -1;
  }

  let end = sceneMarker + ".unity/".length;
  while (end < text.length && !isSceneObjectRefTerminator(text[end])) {
    end++;
  }

  const sceneObjectPath = text.slice(start, end).trimEnd();
  if (!splitSceneObjectRef(sceneObjectPath)) {
    return -1;
  }

  return start + sceneObjectPath.length;
}

function replaceUnquotedSceneObjectRefs(
  text: string,
  render: (path: string) => string,
): string {
  let result = "";
  let cursor = 0;
  UNQUOTED_SCENE_OBJECT_START_RE.lastIndex = 0;

  let match: RegExpExecArray | null;
  while ((match = UNQUOTED_SCENE_OBJECT_START_RE.exec(text)) !== null) {
    const markerStart = match.index;
    const pathStart = markerStart + 1;
    const end = findUnitySceneObjectPathEnd(text, pathStart);
    if (end < 0) continue;
    const sceneObjectPath = text.slice(pathStart, end).trimEnd();

    result += text.slice(cursor, markerStart);
    result += render(sceneObjectPath);
    cursor = end;
    UNQUOTED_SCENE_OBJECT_START_RE.lastIndex = end;
  }

  return result + text.slice(cursor);
}

function isUnityAssetPathBoundaryAt(text: string, index: number): boolean {
  const ch = text[index];
  if (!ch) return true;
  if (ch === ":" && /\d/.test(text[index + 1] ?? "")) return false;
  return /[\s\r\n<>"'`，。；、？！,;:\])}）】》」』]/.test(ch);
}

function readUnityFileIdSuffixEnd(text: string, start: number): number {
  const suffix = text.slice(start).match(/^#fileID:-?\d+/i);
  return suffix ? start + suffix[0].length : start;
}

export function findUnityAssetPathEnd(text: string, start: number): number {
  const lower = text.toLowerCase();
  let bestEnd = -1;

  for (const extension of UNITY_ASSET_ICON_FILE_EXTENSIONS) {
    let searchStart = start;
    while (searchStart < text.length) {
      const extStart = lower.indexOf(extension, searchStart);
      if (extStart < 0) break;
      const extEnd = extStart + extension.length;
      const end = readUnityFileIdSuffixEnd(text, extEnd);
      if (isUnityAssetPathBoundaryAt(text, end)) {
        if (bestEnd < 0 || end < bestEnd) {
          bestEnd = end;
        }
        break;
      }
      searchStart = extStart + 1;
    }
  }

  return bestEnd;
}

function renderUnityPathRef(
  filePath: string,
  options: MarkdownUnityObjectRefOptions = {},
): string {
  const normalized = filePath.trim().replace(/\\/g, "/");
  const sceneObjectRef = splitSceneObjectRef(normalized);
  if (sceneObjectRef) {
    return renderUnitySceneObjectRef(`${sceneObjectRef.scenePath}/${sceneObjectRef.objectPath}`, options);
  }
  return renderUnityAssetRef(normalized, "", options);
}

function replaceLooseUnityAssetRefs(
  text: string,
  render: (path: string) => string,
  startRe: RegExp,
  markerOffset: number,
): string {
  let result = "";
  let cursor = 0;
  startRe.lastIndex = 0;

  let match: RegExpExecArray | null;
  while ((match = startRe.exec(text)) !== null) {
    const markerStart = match.index;
    const pathStart = markerStart + markerOffset;
    const end = findUnityAssetPathEnd(text, pathStart);
    if (end < 0) {
      continue;
    }

    const assetPath = text.slice(pathStart, end).trimEnd();
    if (!ASSET_ROOT_RE.test(assetPath)) {
      continue;
    }

    result += text.slice(cursor, markerStart);
    result += render(assetPath);
    cursor = end;
    startRe.lastIndex = end;
  }

  return result + text.slice(cursor);
}

function decodeCodeText(source: string): string {
  return source
    .replace(/&quot;/g, "\"")
    .replace(/&#39;/g, "'")
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

interface ParsedInlineCodeRefText {
  refText: string;
  unityOptions?: MarkdownUnityObjectRefOptions;
}

function parseInlineCodeRefText(source: string): ParsedInlineCodeRefText {
  const decoded = decodeCodeText(source).trim();
  const braced = decoded.match(INLINE_CODE_BRACED_REF_RE);
  let refText = braced ? braced[1].trim() : decoded;
  let unityOptions: MarkdownUnityObjectRefOptions | undefined;

  const prefixed = refText.match(INLINE_CODE_UNITY_REF_PREFIX_RE);
  if (prefixed) {
    const nextOptions = markdownUnityRefOptionsFromToken(prefixed[1]);
    unityOptions = nextOptions ?? { level: "inline" };
    refText = prefixed[2].trim();
  }

  const suffixed = refText.match(INLINE_CODE_UNITY_REF_SUFFIX_RE);
  if (suffixed) {
    const nextOptions = markdownUnityRefOptionsFromToken(suffixed[2]);
    if (nextOptions) {
      unityOptions = {
        ...unityOptions,
        ...nextOptions,
      };
      refText = suffixed[1].trim();
    }
  }

  refText = refText.replace(/^@(?=[^@\r\n]*\/)/, "");
  return { refText, unityOptions };
}

function splitInlineCodePathSuffix(
  source: string,
  allowSingleSegment = false,
): { path: string; line: string } | null {
  const match = source.match(INLINE_CODE_PATH_SUFFIX_RE);
  if (!match) return null;
  const path = match[1].trim().replace(/\\/g, "/");
  if (!allowSingleSegment && !path.includes("/")) return null;
  return {
    path,
    line: match[2] || match[3] || "",
  };
}

function isPotentialInlinePathStatusCandidate(filePath: string): boolean {
  const normalized = normalizeFileRefPath(filePath);
  if (!normalized || /[\r\n<>]/.test(normalized)) return false;
  if (isUsableAbsoluteLocalRefPath(normalized)) return true;
  if (normalized.includes("/")) return true;
  if (hasFileExtension(normalized)) return true;
  return INLINE_CODE_SINGLE_SEGMENT_ROOT_RE.test(normalized);
}

function inlinePathStatusFor(
  filePath: string,
  options: MarkdownRefInjectOptions,
): MarkdownPathStatus | null | undefined {
  const lookup = options.inlinePathStatus;
  if (!lookup) return undefined;
  const key = normalizeMarkdownPathStatusKey(filePath);
  return lookup(key) ?? lookup(filePath) ?? undefined;
}

function resolvedInlinePathStatus(
  filePath: string,
  options: MarkdownRefInjectOptions,
): MarkdownPathStatus | null | undefined {
  const status = inlinePathStatusFor(filePath, options);
  if (status) return status.exists ? status : null;
  return options.requireInlinePathStatus ? null : undefined;
}

function isWorkspaceInlineRefPath(filePath: string): boolean {
  const normalized = normalizeFileRefPath(filePath);
  if (!normalized.includes("/")) return false;
  if (ASSET_ROOT_RE.test(normalized)) return true;
  if (INLINE_WORKSPACE_ROOT_RE.test(normalized)) return true;
  if (normalized.endsWith("/")) return true;
  return INLINE_GENERIC_FILE_PATH_RE.test(normalized);
}

function renderWorkspaceInlineRef(
  filePath: string,
  line = "",
  status?: MarkdownPathStatus | null,
): string {
  const normalizedPath = normalizeFileRefPath(filePath);
  if (line) {
    return renderLocalFileRef(normalizedPath.replace(/\/+$/, ""), line, status?.entryKind);
  }
  return renderWorkspaceMention(normalizedPath, normalizedPath, status);
}

function assetRefFromInlineCode(source: string, options: MarkdownRefInjectOptions = {}): string | null {
  const inlineRef = parseInlineCodeRefText(source);
  const refText = inlineRef.refText;
  if (INLINE_SLASH_COMMAND_RE.test(refText)) {
    return renderInlineCommandRef(refText);
  }

  const parsed = splitInlineCodePathSuffix(refText, !!options.inlinePathStatus || !!options.requireInlinePathStatus);
  if (!parsed) return null;

  const knowledgeRef = parseKnowledgeRefPath(parsed.path);
  if (knowledgeRef) {
    return renderKnowledgeRef(knowledgeRef.path);
  }

  if (isUsableAbsoluteLocalRefPath(parsed.path)) {
    const status = resolvedInlinePathStatus(parsed.path, options);
    if (status === null) return null;
    return renderLocalFileRef(parsed.path, parsed.line, status?.entryKind);
  }

  const sceneObjectRef = splitSceneObjectRef(parsed.path);
  if (sceneObjectRef) {
    const sceneStatus = resolvedInlinePathStatus(sceneObjectRef.scenePath, options);
    if (sceneStatus === null || sceneStatus?.entryKind === "folder") return null;
    return renderUnitySceneObjectRef(`${sceneObjectRef.scenePath}/${sceneObjectRef.objectPath}`, inlineRef.unityOptions);
  }

  const status = resolvedInlinePathStatus(parsed.path, options);
  const hasKnownStatus = status !== undefined;
  if (status === null) return null;
  if (!isWorkspaceInlineRefPath(parsed.path) && !hasKnownStatus) return null;

  if (isUnityAssetRootPath(parsed.path)) {
    return renderWorkspaceInlineRef(parsed.path, parsed.line, status);
  }

  if (ASSET_ROOT_RE.test(parsed.path)) {
    return renderUnityAssetRef(parsed.path, parsed.line, inlineRef.unityOptions, status);
  }

  if (!isPotentialInlinePathStatusCandidate(parsed.path)) return null;
  return renderWorkspaceInlineRef(parsed.path, parsed.line, status);
}

function unityAssetRefFromCodeText(
  source: string,
  defaultUnityOptions?: MarkdownUnityObjectRefOptions,
): string | null {
  const inlineRef = parseInlineCodeRefText(source);
  const parsed = splitInlineCodePathSuffix(inlineRef.refText);
  if (!parsed || !isWorkspaceInlineRefPath(parsed.path)) return null;

  const unityOptions = inlineRef.unityOptions ?? defaultUnityOptions;
  if (!unityOptions && inlineRef.refText === source.trim()) return null;

  const sceneObjectRef = splitSceneObjectRef(parsed.path);
  if (sceneObjectRef) {
    return renderUnitySceneObjectRef(`${sceneObjectRef.scenePath}/${sceneObjectRef.objectPath}`, unityOptions);
  }

  if (ASSET_ROOT_RE.test(parsed.path)) {
    return renderUnityAssetRef(parsed.path, parsed.line, unityOptions);
  }

  return null;
}

function decodeHtmlTextFragment(source: string): string {
  return decodeCodeText(source
    .replace(/<span\b[^>]*\bline-number\b[^>]*>[\s\S]*?<\/span>/gi, "")
    .replace(/<[^>]+>/g, ""));
}

function unityObjectFenceLanguageFromAttrs(attrs: string): string {
  const match = attrs.match(CODE_CLASS_LANGUAGE_RE);
  return match?.[1] ? decodeCodeText(match[1]) : "";
}

export function injectUnityPropertyFenceRefs(html: string): string {
  return html.replace(CODE_BLOCK_RE, (match, attrs: string, content: string) => {
    if (!isMarkdownUnityPropertyFenceLanguage(unityObjectFenceLanguageFromAttrs(attrs))) return match;
    const source = decodeHtmlTextFragment(content).trim();
    if (!source) return match;

    return [
      '<div class="md-unity-property-fence-host" data-md-unity-property-fence="true">',
      '<pre class="md-unity-property-fallback"><code data-md-unity-property-source="true">',
      escapeAttr(source),
      "</code></pre>",
      "</div>",
    ].join("");
  });
}

export function injectUnityObjectFenceRefs(html: string): string {
  return html.replace(CODE_BLOCK_RE, (match, attrs: string, content: string) => {
    const languageOptions = markdownUnityRefOptionsFromFenceLanguage(unityObjectFenceLanguageFromAttrs(attrs));
    const source = decodeHtmlTextFragment(content).trim();
    if (!source) return match;

    const lines = source.split(/\r?\n/g).map((line) => line.trim()).filter(Boolean);
    if (!lines.length) return match;

    const refs = lines.map((line) => unityAssetRefFromCodeText(line, languageOptions ?? undefined));
    if (refs.some((ref) => !ref)) return match;
    return refs.join("");
  });
}

function collectInlineCodePathCandidatesFromSource(source: string): string[] {
  const inlineRef = parseInlineCodeRefText(source);
  const refText = inlineRef.refText;
  if (INLINE_SLASH_COMMAND_RE.test(refText)) return [];

  const parsed = splitInlineCodePathSuffix(refText, true);
  if (!parsed) return [];

  const knowledgeRef = parseKnowledgeRefPath(parsed.path);
  if (knowledgeRef) return [];

  const sceneObjectRef = splitSceneObjectRef(parsed.path);
  if (sceneObjectRef) return [normalizeMarkdownPathStatusKey(sceneObjectRef.scenePath)];

  if (!isPotentialInlinePathStatusCandidate(parsed.path)) return [];
  return [normalizeMarkdownPathStatusKey(parsed.path)];
}

export function collectInlineCodePathCandidates(html: string): string[] {
  const parts = html.split(/(<[^>]+>)/);
  const candidates: string[] = [];
  const seen = new Set<string>();
  let inPre = 0;
  let inAnchor = 0;

  const addCandidate = (path: string) => {
    const key = normalizeMarkdownPathStatusKey(path);
    if (!key || seen.has(key)) return;
    seen.add(key);
    candidates.push(key);
  };

  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    if (!part.startsWith("<")) continue;

    if (/^<pre[\s>]/i.test(part)) {
      inPre++;
      continue;
    }
    if (/^<\/pre>/i.test(part)) {
      inPre = Math.max(0, inPre - 1);
      continue;
    }
    if (/^<a[\s>]/i.test(part)) {
      inAnchor++;
      continue;
    }
    if (/^<\/a>/i.test(part)) {
      inAnchor = Math.max(0, inAnchor - 1);
      continue;
    }

    if (inPre > 0 || inAnchor > 0) continue;
    if (!/^<code[\s>]/i.test(part)) continue;
    if (!parts[i + 2] || !/^<\/code>/i.test(parts[i + 2])) continue;

    for (const candidate of collectInlineCodePathCandidatesFromSource(parts[i + 1] || "")) {
      addCandidate(candidate);
    }
  }

  return candidates;
}

function injectInlineCodeAssetRefs(html: string, options: MarkdownRefInjectOptions = {}): string {
  const parts = html.split(/(<[^>]+>)/);
  let inPre = 0;
  let inAnchor = 0;
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    if (!part.startsWith("<")) continue;

    if (/^<pre[\s>]/i.test(part)) {
      inPre++;
      continue;
    }
    if (/^<\/pre>/i.test(part)) {
      inPre = Math.max(0, inPre - 1);
      continue;
    }
    if (/^<a[\s>]/i.test(part)) {
      inAnchor++;
      continue;
    }
    if (/^<\/a>/i.test(part)) {
      inAnchor = Math.max(0, inAnchor - 1);
      continue;
    }

    if (inPre > 0 || inAnchor > 0) continue;
    if (!/^<code[\s>]/i.test(part)) continue;
    if (!parts[i + 2] || !/^<\/code>/i.test(parts[i + 2])) continue;

    const ref = assetRefFromInlineCode(parts[i + 1] || "", options);
    if (!ref) continue;
    parts.splice(i, 3, ref);
  }
  return parts.join("");
}

export function injectAssetRefs(html: string, options: MarkdownRefInjectOptions = {}): string {
  const injectedTextRefs = walkHtmlText(html, (text) => {
    const refs: string[] = [];
    const stashRef = (refHtml: string) => {
      const key = `\u0000mdref:${refs.length}\u0000`;
      refs.push(refHtml);
      return key;
    };

    const delimitedSceneRefsInjected = text
      .replace(QUOTED_SCENE_OBJECT_REF_RE, (_match, _quote, path) => stashRef(renderUnitySceneObjectRef(path)))
      .replace(BRACED_UNITY_REF_RE, (_match, path) => stashRef(renderUnityPathRef(path)));

    const sceneRefsInjected = replaceUnquotedSceneObjectRefs(
      delimitedSceneRefsInjected,
      (path) => stashRef(renderUnitySceneObjectRef(path)),
    );

    const delimitedRefsInjected = sceneRefsInjected
      .replace(PARENTHESIZED_UNITY_ASSET_REF_RE, (_match, path) => stashRef(renderUnityAssetRef(path)))
      .replace(QUOTED_ASSET_REF_RE, (_match, _quote, path) => stashRef(renderUnityAssetRef(path)));

    const injected = replaceLooseUnityAssetRefs(
      delimitedRefsInjected,
      (path) => stashRef(renderUnityAssetRef(path)),
      UNQUOTED_UNITY_ASSET_START_RE,
      1,
    )
      .replace(ASSET_REF_RE, (_match, path) => stashRef(renderUnityAssetRef(path)));

    return injected.replace(/\u0000mdref:(\d+)\u0000/g, (_match, index) => refs[Number(index)] ?? "");
  });
  return injectInlineCodeAssetRefs(injectedTextRefs, options);
}

export function injectAssetChips(html: string, options: MarkdownRefInjectOptions = {}): string {
  return injectAssetRefs(html, options);
}

export function injectWorkspaceMentions(html: string): string {
  return walkHtmlText(html, (text) => {
    const refs: string[] = [];
    const stashRef = (refHtml: string) => {
      const key = `\u0000mdref:${refs.length}\u0000`;
      refs.push(refHtml);
      return key;
    };

    const braced = text.replace(BRACED_WORKSPACE_MENTION_RE, (match, path) =>
      stashRef(renderWorkspaceMention(path, match)),
    );

    const injected = braced.replace(WORKSPACE_MENTION_RE, (match, path) =>
      stashRef(renderWorkspaceMention(path, match)),
    );

    return injected.replace(/\u0000mdref:(\d+)\u0000/g, (_match, index) => refs[Number(index)] ?? "");
  });
}

// Match project-relative file paths, optionally with :line or #Lline suffix.
// Requires at least one slash and a file extension to reduce false positives.
// Does not match if preceded by @ (already handled as an asset/workspace mention) or backticks.
const FILE_REF_RE = /(?<![@`\/\w])(?:(?:src|src-tauri|Assets|Packages|Library|ProjectSettings|Editor)\/[\w.\/\-]+[\w.\-]|[\w.\-]+\/[\w.\/\-]*\.[\w]+)(?::(\d+)|#L(\d+))?/g;

// Detects if a match is inside a URL by checking preceding text for ://
const URL_CONTEXT_RE = /\w+:\/\/\S*$/;
const URL_PROTOCOL_PREFIX_RE = /\w+:$/;

function splitTrailingFileRefPunctuation(source: string): { value: string; trailing: string } {
  const match = source.match(TRAILING_FILE_REF_PUNCT_RE);
  if (!match) return { value: source, trailing: "" };
  return {
    value: source.slice(0, -match[0].length),
    trailing: match[0],
  };
}

function renderAbsoluteLocalFileRefCandidate(source: string): string | null {
  const { value, trailing } = splitTrailingFileRefPunctuation(source);
  const parsed = splitInlineCodePathSuffix(value);
  if (!parsed || !isUsableAbsoluteLocalRefPath(parsed.path)) return null;
  return `${renderLocalFileRef(parsed.path, parsed.line)}${trailing}`;
}

export function injectFileRefs(html: string): string {
  return walkHtmlText(html, (text) => {
    // Skip text inside already-injected refs.
    if (text.includes("data-asset-path") || text.includes("data-workspace-path")) return text;
    const refs: string[] = [];
    const stashRef = (refHtml: string) => {
      const key = `\u0000mdref:${refs.length}\u0000`;
      refs.push(refHtml);
      return key;
    };

    const looseUnityRefs = replaceLooseUnityAssetRefs(
      text,
      (path) => stashRef(renderUnityPathRef(path)),
      BARE_UNITY_ASSET_START_RE,
      0,
    );

    const quotedLocalRefs = looseUnityRefs.replace(QUOTED_LOCAL_FILE_REF_RE, (match, _quote, path) => {
      const rendered = renderAbsoluteLocalFileRefCandidate(path);
      return rendered ? stashRef(rendered) : match;
    });

    const localRefs = quotedLocalRefs.replace(ABSOLUTE_LOCAL_FILE_REF_RE, (match, path, offset, fullText) => {
      const preceding = fullText.slice(0, offset);
      if (URL_CONTEXT_RE.test(preceding) || URL_PROTOCOL_PREFIX_RE.test(preceding)) return match;
      const rendered = renderAbsoluteLocalFileRefCandidate(path);
      return rendered ? stashRef(rendered) : match;
    });

    const injected = localRefs.replace(FILE_REF_RE, (match, lineColon, lineHash, offset, fullText) => {
      // Skip matches that are part of a URL
      const preceding = fullText.slice(0, offset);
      if (URL_CONTEXT_RE.test(preceding)) return match;
      const line = lineColon || lineHash || "";
      // Strip line suffix to get clean file path
      let filePath = match;
      if (lineColon) filePath = match.slice(0, match.lastIndexOf(":" + lineColon));
      else if (lineHash) filePath = match.slice(0, match.lastIndexOf("#L" + lineHash));
      const knowledgeRef = parseKnowledgeRefPath(filePath);
      if (knowledgeRef) {
        return renderKnowledgeRef(knowledgeRef.path);
      }
      if (SCENE_OBJECT_ROOT_RE.test(filePath)) {
        return renderUnitySceneObjectRef(filePath);
      }
      if (ASSET_ROOT_RE.test(filePath)) {
        return renderUnityAssetRef(filePath, line);
      }
      return renderLocalFileRef(filePath, line);
    });
    return injected.replace(/\u0000mdref:(\d+)\u0000/g, (_match, index) => refs[Number(index)] ?? "");
  });
}
