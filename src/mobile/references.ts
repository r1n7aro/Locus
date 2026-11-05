import type { KnowledgeDocumentType } from "../types";
import { normalizeExternalMarkdownHref } from "../composables/markdownExternalLinks";
import type { DocumentSource } from "./routes";

// References and links inside rendered Markdown (conversations and opened
// documents), resolved to what the phone opens for them.

export const KNOWLEDGE_TYPES: readonly KnowledgeDocumentType[] = ["design", "plan", "memory", "reference", "skill"];
/** Project knowledge lives in the workspace under this folder. */
export const PROJECT_KNOWLEDGE_ROOT = "Locus/knowledge";

export type ReferenceTarget =
  | { kind: "file"; path: string }
  | { kind: "folder"; path: string }
  | { kind: "knowledge"; docType: KnowledgeDocumentType; path: string };

export type LinkTarget = ReferenceTarget | { kind: "anchor"; id: string } | { kind: "external" };

function slashes(path: string): string {
  return path.trim().replace(/\\/g, "/");
}

function trimTrailingSlash(path: string): string {
  return path.length > 1 ? path.replace(/\/+$/, "") : path;
}

export function knowledgeType(value: string | undefined): KnowledgeDocumentType | null {
  const normalized = (value ?? "").trim().toLowerCase();
  return (KNOWLEDGE_TYPES as readonly string[]).includes(normalized) ? normalized as KnowledgeDocumentType : null;
}

export function isAbsolutePath(path: string): boolean {
  return /^(?:[A-Za-z]:\/|\/\/|\/)/.test(slashes(path));
}

/** Collapses `.` and `..`; a relative path that climbs above its start keeps
 * the leading `..` so callers can refuse it. */
export function normalizePath(path: string): string {
  const value = slashes(path);
  const drive = /^[A-Za-z]:\//.exec(value)?.[0] ?? (value.startsWith("//") ? "//" : value.startsWith("/") ? "/" : "");
  const parts: string[] = [];
  for (const part of value.slice(drive.length).split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (parts.length && parts[parts.length - 1] !== "..") parts.pop();
      else if (!drive) parts.push("..");
      continue;
    }
    parts.push(part);
  }
  return drive + parts.join("/");
}

/** Directory of `path`; absolute roots keep their slash ("/", "C:/"). */
function parentDir(path: string): string {
  const normalized = normalizePath(path);
  const index = normalized.lastIndexOf("/");
  if (index < 0) return "";
  const dir = normalized.slice(0, index + 1);
  return /^(?:\/|\/\/|[A-Za-z]:\/)$/.test(dir) ? dir : dir.slice(0, -1);
}

function joinPath(dir: string, relative: string): string {
  if (!dir) return relative;
  return dir.endsWith("/") ? `${dir}${relative}` : `${dir}/${relative}`;
}

/** `Locus/knowledge/<type>/<path>` (Markdown or CSV) and `<type>/<path>.md`
 * name knowledge documents, as on the desktop. */
export function knowledgeTarget(path: string): { docType: KnowledgeDocumentType; path: string } | null {
  const normalized = trimTrailingSlash(slashes(path)).replace(/^\/+/, "");
  const prefix = `${PROJECT_KNOWLEDGE_ROOT}/`;
  const inProject = normalized.toLowerCase().startsWith(prefix.toLowerCase());
  const relative = inProject ? normalized.slice(prefix.length) : normalized;
  const match = (inProject ? /^([a-z]+)\/(.+\.(?:md|csv))$/i : /^([a-z]+)\/(.+\.md)$/i).exec(relative);
  const docType = knowledgeType(match?.[1]);
  if (!match || !docType) return null;
  return { docType, path: `${docType}/${match[2]}` };
}

function pathTarget(path: string | undefined, entryKind: string | undefined): ReferenceTarget | null {
  const value = trimTrailingSlash(slashes(path ?? ""));
  if (!value) return null;
  const knowledge = knowledgeTarget(value);
  if (knowledge) return { kind: "knowledge", ...knowledge };
  return { kind: entryKind === "folder" ? "folder" : "file", path: value };
}

/** The file, folder or knowledge reference rendered at `element`, if any
 * (the reference markup `MarkdownRenderer` injects). */
export function referenceFromElement(element: Element): ReferenceTarget | null {
  const knowledge = element.closest<HTMLElement>(".md-knowledge-ref[data-knowledge-path]");
  if (knowledge) {
    const path = trimTrailingSlash(slashes(knowledge.dataset.knowledgePath ?? ""));
    const docType = knowledgeType(knowledge.dataset.knowledgeType) ?? knowledgeType(path.split("/")[0]);
    return docType && path ? { kind: "knowledge", docType, path } : null;
  }
  const workspace = element.closest<HTMLElement>(".md-workspace-ref[data-workspace-path]");
  if (workspace) return pathTarget(workspace.dataset.workspacePath, workspace.dataset.entryKind);
  const chip = element.closest<HTMLElement>(".md-asset-chip[data-asset-path], .md-unity-asset-ref[data-asset-path]");
  if (chip) return pathTarget(chip.dataset.assetPath, chip.dataset.entryKind);
  const file = element.closest<HTMLElement>(".md-file-ref[data-file-path]");
  if (file) return pathTarget(file.dataset.filePath, file.dataset.entryKind);
  return null;
}

function knowledgeRelative(docType: KnowledgeDocumentType, path: string): string {
  const normalized = trimTrailingSlash(slashes(path)).replace(/^\/+/, "");
  return normalized.toLowerCase().startsWith(`${docType}/`) ? normalized.slice(docType.length + 1) : normalized;
}

/** Workspace path of a project knowledge document. */
export function knowledgeFilePath(docType: KnowledgeDocumentType, path: string): string {
  return `${PROJECT_KNOWLEDGE_ROOT}/${docType}/${knowledgeRelative(docType, path)}`;
}

function decode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

/** What a link inside an opened document points at. Relative links resolve
 * against the document; links that leave the workspace resolve to nothing. */
export function resolveDocumentLink(rawHref: string | null | undefined, source: DocumentSource): LinkTarget | null {
  const href = rawHref?.trim();
  if (!href) return null;
  if (href.startsWith("#")) return { kind: "anchor", id: decode(href.slice(1)) };
  if (normalizeExternalMarkdownHref(href)) return { kind: "external" };
  // Other schemes (file:, javascript:, data:, ...) are not followed.
  if (/^[a-z][a-z0-9+.-]*:/i.test(href) && !/^[A-Za-z]:[\\/]/.test(href)) return null;
  const [withoutFragment] = href.split("#", 1);
  const target = decode((withoutFragment ?? "").split("?", 1)[0] ?? "");
  if (!target) return null;
  const folder = /\/$/.test(target);

  if (source.kind === "knowledge") {
    const base = `${source.docType}/${knowledgeRelative(source.docType, source.path)}`;
    const resolved = normalizePath(target.startsWith("/") ? target.slice(1) : joinPath(parentDir(base), target));
    const docType = knowledgeType(resolved.split("/")[0]);
    if (docType && /\.(?:md|csv)$/i.test(resolved) && !folder) return { kind: "knowledge", docType, path: resolved };
    // Assets next to project knowledge, or links out into the workspace.
    const file = normalizePath(`${PROJECT_KNOWLEDGE_ROOT}/${resolved}`);
    if (file.startsWith("..")) return null;
    return { kind: folder ? "folder" : "file", path: trimTrailingSlash(file) };
  }

  const resolved = isAbsolutePath(target)
    ? (/^\/[^/]/.test(target) && !isAbsolutePath(source.path) ? normalizePath(target.slice(1)) : normalizePath(target))
    : normalizePath(joinPath(parentDir(source.path), target));
  if (!resolved || resolved.startsWith("..")) return null;
  const knowledge = folder ? null : knowledgeTarget(resolved);
  if (knowledge) return { kind: "knowledge", ...knowledge };
  return { kind: folder ? "folder" : "file", path: trimTrailingSlash(resolved) };
}

/** `path` relative to `root` when it lies inside it (case-insensitive, as
 * on Windows); relative paths are returned unchanged. */
export function relativeToRoot(path: string, root: string | null | undefined): string | null {
  const normalized = normalizePath(path);
  if (!isAbsolutePath(normalized)) return normalized.startsWith("..") ? null : normalized;
  if (!root) return null;
  const base = trimTrailingSlash(normalizePath(root));
  if (normalized.toLowerCase() === base.toLowerCase()) return "";
  return normalized.toLowerCase().startsWith(`${base.toLowerCase()}/`) ? normalized.slice(base.length + 1) : null;
}

/** GitHub-style heading slug, used to follow `#anchor` links. */
export function headingSlug(text: string): string {
  return text
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .replace(/\s/g, "-");
}
