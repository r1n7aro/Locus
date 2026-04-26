/**
 * Pure functions for injecting interactive elements (asset chips, file refs)
 * into rendered Markdown HTML. Extracted for testability.
 */

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
const QUOTED_ASSET_REF_RE = /(["'])@?((?:Assets|Packages)\/[\w.\/-]*[\w.-]\/?)\s*\1/g;
const ASSET_REF_RE = /@((?:Assets|Packages)\/[\w.\/-]*[\w.-])(?!\/)/g;
const INLINE_CODE_ASSET_REF_RE = /^@?((?:Assets|Packages)\/[\w.\/-]*[\w.-]\/?)(?::(\d+)|#L(\d+))?$/;
const UNQUOTED_SCENE_OBJECT_START_RE = /@(?:Assets|Packages)\//g;
const WORKSPACE_MENTION_RE = /@((?:[^\s@<]+\/)+[^\s@<]*)/g;
const UNITY_ASSET_ICON_BASE = "/unity-asset-icons";

type UnityAssetKind =
  | "scene"
  | "prefab"
  | "material"
  | "script"
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
  | "asset";

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

function displaySceneObjectRef(objectPath: string): string {
  const normalized = objectPath.replace(/\/+$/, "") || objectPath;
  const segments = normalized.split("/").filter(Boolean);
  return segments[segments.length - 1] || normalized;
}

function unityAssetKind(filePath: string): UnityAssetKind {
  const normalized = filePath.replace(/\/+$/, "");
  const fileName = (normalized.split("/").pop() || normalized || filePath).toLowerCase();
  if (filePath.endsWith("/") || !fileName.includes(".")) return "folder";
  if (fileName.endsWith(".unity")) return "scene";
  if (fileName.endsWith(".prefab")) return "prefab";
  if (
    fileName.endsWith(".mat")
    || fileName.endsWith(".physicmaterial")
    || fileName.endsWith(".physicsmaterial2d")
  ) return "material";
  if (fileName.endsWith(".cs") || fileName.endsWith(".asmdef") || fileName.endsWith(".asmref")) {
    return "script";
  }
  if (
    fileName.endsWith(".shader")
    || fileName.endsWith(".shadergraph")
    || fileName.endsWith(".compute")
    || fileName.endsWith(".hlsl")
    || fileName.endsWith(".cginc")
  ) return "shader";
  if (
    /\.(png|jpe?g|tga|psd|bmp|gif|tiff?|exr|hdr|dds|svg|webp)$/.test(fileName)
  ) return "texture";
  if (/\.(fbx|obj|blend|dae|3ds|ma|mb|max)$/.test(fileName)) return "model";
  if (/\.(anim|controller|overridecontroller|mask)$/.test(fileName)) return "animation";
  if (/\.(wav|mp3|ogg|aiff?|flac|xm|mod|it|s3m)$/.test(fileName)) return "audio";
  if (/\.(ttf|otf|fontsettings)$/.test(fileName)) return "font";
  if (/\.(mp4|mov|webm|avi|mpeg|mpg)$/.test(fileName)) return "video";
  if (/\.(txt|md|json|xml|ya?ml|csv|bytes|uxml|uss)$/.test(fileName)) return "text";
  if (fileName.endsWith(".meta")) return "meta";
  return "asset";
}

function unityAssetIconSrc(kind: UnityAssetKind | "file" | "folder"): string {
  return `${UNITY_ASSET_ICON_BASE}/${kind}.svg`;
}

function renderRefIcon(kind: UnityAssetKind | "file" | "folder" = "file", classes = ""): string {
  const className = ["md-ref-icon", classes].filter(Boolean).join(" ");
  return `<img class="${className}" src="${unityAssetIconSrc(kind)}" alt="" aria-hidden="true" draggable="false" loading="lazy">`;
}

function renderUnityAssetIcon(kind: UnityAssetKind): string {
  return renderRefIcon(kind, `md-unity-asset-icon md-unity-asset-icon--${kind}`);
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
  const title = `${escaped}${line ? ":" + escapeAttr(line) : ""}`;
  const className = ["md-file-ref", classes, "ui-select-text"].filter(Boolean).join(" ");
  return `<span class="${className}" data-file-path="${escaped}"${lineAttr}${attrs} title="${title}">${icon}<span class="md-ref-label">${displayFileRef(filePath, line)}</span></span>`;
}

function renderUnityAssetRef(filePath: string, line = ""): string {
  const normalizedPath = filePath.replace(/\/+$/, "") || filePath;
  const escaped = escapeAttr(normalizedPath);
  const kind = unityAssetKind(filePath);
  return renderFileRef(
    normalizedPath,
    line,
    "md-unity-asset-ref",
    ` data-asset-path="${escaped}" data-asset-kind="${kind}"`,
    renderUnityAssetIcon(kind),
  );
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

function renderUnitySceneObjectRef(filePath: string): string {
  const ref = splitSceneObjectRef(filePath);
  if (!ref) return escapeAttr(filePath);
  const fullPath = `${ref.scenePath}/${ref.objectPath}`;
  const escapedFullPath = escapeAttr(fullPath);
  const escapedScenePath = escapeAttr(ref.scenePath);
  const escapedObjectPath = escapeAttr(ref.objectPath);
  const escapedLabel = escapeAttr(displaySceneObjectRef(ref.objectPath));
  const icon = renderRefIcon("gameobject", "md-unity-gameobject-icon");
  return `<span class="md-file-ref md-unity-scene-object-ref ui-select-text" data-file-path="${escapedFullPath}" data-scene-path="${escapedScenePath}" data-scene-object-path="${escapedObjectPath}" title="${escapedFullPath}">${icon}<span class="md-ref-label">${escapedLabel}</span></span>`;
}

function isSceneObjectRefTerminator(ch: string): boolean {
  return /[\r\n<>"'`，。；、？！]/.test(ch);
}

function replaceUnquotedSceneObjectRefs(
  text: string,
  render: (path: string) => string,
): string {
  let result = "";
  let cursor = 0;
  const lower = text.toLowerCase();
  UNQUOTED_SCENE_OBJECT_START_RE.lastIndex = 0;

  let match: RegExpExecArray | null;
  while ((match = UNQUOTED_SCENE_OBJECT_START_RE.exec(text)) !== null) {
    const markerStart = match.index;
    const pathStart = markerStart + 1;
    const sceneMarker = lower.indexOf(".unity/", pathStart);
    if (sceneMarker < 0 || text.slice(pathStart, sceneMarker).includes("@")) {
      continue;
    }

    let end = sceneMarker + ".unity/".length;
    while (end < text.length && !isSceneObjectRefTerminator(text[end])) {
      end++;
    }

    const sceneObjectPath = text.slice(pathStart, end).trimEnd();
    if (!splitSceneObjectRef(sceneObjectPath)) {
      continue;
    }

    result += text.slice(cursor, markerStart);
    result += render(sceneObjectPath);
    cursor = end;
    UNQUOTED_SCENE_OBJECT_START_RE.lastIndex = end;
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

function assetRefFromInlineCode(source: string): string | null {
  const decoded = decodeCodeText(source).trim();
  const sceneObjectRef = splitSceneObjectRef(decoded.replace(/^@/, ""));
  if (sceneObjectRef) return renderUnitySceneObjectRef(`${sceneObjectRef.scenePath}/${sceneObjectRef.objectPath}`);
  const match = decoded.match(INLINE_CODE_ASSET_REF_RE);
  if (!match) return null;
  const [, filePath, lineColon, lineHash] = match;
  return renderUnityAssetRef(filePath, lineColon || lineHash || "");
}

function injectInlineCodeAssetRefs(html: string): string {
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

    const ref = assetRefFromInlineCode(parts[i + 1] || "");
    if (!ref) continue;
    parts.splice(i, 3, ref);
  }
  return parts.join("");
}

export function injectAssetRefs(html: string): string {
  const injectedTextRefs = walkHtmlText(html, (text) => {
    const refs: string[] = [];
    const stashRef = (refHtml: string) => {
      const key = `\u0000mdref:${refs.length}\u0000`;
      refs.push(refHtml);
      return key;
    };

    const sceneRefsInjected = replaceUnquotedSceneObjectRefs(
      text.replace(QUOTED_SCENE_OBJECT_REF_RE, (_match, _quote, path) => stashRef(renderUnitySceneObjectRef(path))),
      (path) => stashRef(renderUnitySceneObjectRef(path)),
    );

    const injected = sceneRefsInjected
      .replace(QUOTED_ASSET_REF_RE, (_match, _quote, path) => stashRef(renderUnityAssetRef(path)))
      .replace(ASSET_REF_RE, (_match, path) => stashRef(renderUnityAssetRef(path)));

    return injected.replace(/\u0000mdref:(\d+)\u0000/g, (_match, index) => refs[Number(index)] ?? "");
  });
  return injectInlineCodeAssetRefs(injectedTextRefs);
}

export function injectAssetChips(html: string): string {
  return injectAssetRefs(html);
}

export function injectWorkspaceMentions(html: string): string {
  return walkHtmlText(html, (text) =>
    text.replace(WORKSPACE_MENTION_RE, (match, path) => {
      const isDir = path.endsWith("/");
      if (/^(Assets|Packages|ProjectSettings)\//.test(path) && !isDir) {
        return match;
      }

      const normalizedPath = path.replace(/\/+$/, "");
      if (!normalizedPath) {
        return match;
      }

      const escapedPath = normalizedPath.replace(/"/g, "&quot;");
      const segments = normalizedPath.split("/").filter(Boolean);
      const name = segments[segments.length - 1] || normalizedPath;
      const title = `${escapedPath}${isDir ? "/" : ""}`;
      const fileAttr = isDir ? "" : ` data-file-path="${escapedPath}"`;
      const classes = isDir ? "md-workspace-ref md-folder-ref" : "md-workspace-ref md-file-ref";
      const icon = isDir ? renderRefIcon("folder", "md-workspace-ref-icon") : "";

      return `<span class="${classes} ui-select-text" data-workspace-path="${escapedPath}" data-entry-kind="${isDir ? "folder" : "file"}"${fileAttr} title="${title}">${icon}<span class="md-workspace-ref-prefix">@</span>${name}${isDir ? "/" : ""}</span>`;
    }),
  );
}

// Match project-relative file paths, optionally with :line or #Lline suffix.
// Requires at least one slash and a file extension to reduce false positives.
// Does not match if preceded by @ (already handled as an asset/workspace mention) or backticks.
const FILE_REF_RE = /(?<![@`\/])(?:(?:src|src-tauri|Assets|Packages|Library|ProjectSettings|Editor)\/[\w.\/\-]+[\w.\-]|[\w.\-]+\/[\w.\/\-]*\.[\w]+)(?::(\d+)|#L(\d+))?/g;

// Detects if a match is inside a URL by checking preceding text for ://
const URL_CONTEXT_RE = /\w+:\/\/\S*$/;

export function injectFileRefs(html: string): string {
  return walkHtmlText(html, (text) => {
    // Skip text inside already-injected refs.
    if (text.includes("data-asset-path") || text.includes("data-workspace-path")) return text;
    return text.replace(FILE_REF_RE, (match, lineColon, lineHash, offset, fullText) => {
      // Skip matches that are part of a URL
      const preceding = fullText.slice(0, offset);
      if (URL_CONTEXT_RE.test(preceding)) return match;
      const line = lineColon || lineHash || "";
      // Strip line suffix to get clean file path
      let filePath = match;
      if (lineColon) filePath = match.slice(0, match.lastIndexOf(":" + lineColon));
      else if (lineHash) filePath = match.slice(0, match.lastIndexOf("#L" + lineHash));
      if (SCENE_OBJECT_ROOT_RE.test(filePath)) {
        return renderUnitySceneObjectRef(filePath);
      }
      if (ASSET_ROOT_RE.test(filePath)) {
        return renderUnityAssetRef(filePath, line);
      }
      return renderFileRef(filePath, line);
    });
  });
}
