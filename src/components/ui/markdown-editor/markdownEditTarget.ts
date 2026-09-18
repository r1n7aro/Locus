import type { ChangeDesc, EditorState } from "@codemirror/state";
import type { MarkdownReferenceToken } from "./markdownComplexTokens";
import { markdownNodeAt } from "./markdownVisualCommands";

export interface MarkdownEditTarget {
  kind: "link" | "image" | "reference";
  from: number;
  to: number;
  source: string;
  label: string;
  url: string;
  title?: string;
  reference?: MarkdownReferenceToken;
}

export function decodeMarkdownDestination(source: string): string {
  return source.replace(/^<|>$/g, "").replace(/\\([\\()[\]<>])/g, "$1");
}

export function markdownLinkTarget(state: EditorState, position: number): MarkdownEditTarget | null {
  const node = markdownNodeAt(state, position, ["Link"]);
  const url = node?.getChild("URL");
  const marks = node?.getChildren("LinkMark");
  if (!node || !url || !marks || marks.length < 2) return null;
  const title = node.getChild("LinkTitle");
  return {
    kind: "link", from: node.from, to: node.to,
    source: state.sliceDoc(node.from, node.to),
    label: state.sliceDoc(marks[0].to, marks[1].from),
    url: decodeMarkdownDestination(state.sliceDoc(url.from, url.to)),
    title: title ? state.sliceDoc(title.from + 1, title.to - 1) : "",
  };
}

export function mapMarkdownEditTarget(target: MarkdownEditTarget, changes: ChangeDesc): MarkdownEditTarget | null {
  let touched = false;
  changes.iterChangedRanges((from, to) => {
    if (from < target.to && to > target.from
      || from === to && from > target.from && from < target.to
      || target.from === target.to && from <= target.from && to >= target.to) touched = true;
  });
  if (touched) return null;
  return { ...target, from: changes.mapPos(target.from, 1), to: changes.mapPos(target.to, -1) };
}

function markdownDestination(value: string): string {
  const path = value.trim().replace(/\\/g, "/").replace(/[<>]/g, (char) => encodeURIComponent(char));
  // Angle destinations preserve spaces in local paths for the workspace image
  // resolver. Percent-encoding them would address a different on-disk file.
  return /\s/.test(path) ? `<${path}>` : path.replace(/[()]/g, (char) => `\\${char}`);
}

export function serializeMarkdownEdit(target: MarkdownEditTarget, url: string, label: string): string {
  const path = url.trim();
  if (!path || /[\r\n]/.test(path) || /^(?:javascript|vbscript):/i.test(path)
    || /^data:/i.test(path) && !(target.kind === "image" && /^data:image\//i.test(path))) throw new Error("请输入有效地址");
  if (path === target.url && label === target.label) return target.source;
  if (target.kind === "reference") {
    const reference = target.reference!;
    // Replace the target only; preserve modes, line suffixes, fence syntax and
    // property selectors instead of serializing an entire reference block.
    const oldPath = reference.path;
    if (reference.kind === "unity-property" && /^[\[{]/.test(target.source.trim())) {
      try {
        const json = JSON.parse(target.source);
        const items = Array.isArray(json) ? json : Array.isArray(json.properties) ? json.properties : [json];
        const first = items[0];
        if (first && typeof first === "object") {
          const value = first.target && typeof first.target === "object" ? first.target : first;
          const scene = path.match(/^(.*?\.unity)\/(.+)$/i);
          if (scene) { value.scenePath = scene[1]; value.objectPath = scene[2]; }
          else value.path = path;
          // File IDs belong to the old object. Resolve the newly selected path
          // while retaining component and property selectors.
          for (const key of ["guid", "objectFileId", "gameObjectFileId", "fileId", "targetFileId"]) delete value[key];
          return JSON.stringify(json, null, target.source.includes("\n") ? 2 : undefined);
        }
      } catch { /* Compact property syntax follows the path replacement below. */ }
    }
    const index = target.source.replace(/\\/g, "/").indexOf(oldPath);
    if (index >= 0) {
      const source = target.source.slice(0, index) + path + target.source.slice(index + oldPath.length);
      return reference.kind !== "unity-property" && /\s/.test(path) && !/^[`{]/.test(source) ? `\`${source}\`` : source;
    }
    throw new Error("此引用请在源码模式修改");
  }
  const text = target.kind === "link" ? target.label : label.replace(/[\r\n]/g, " ").replace(/([\\\[\]])/g, "\\$1");
  const title = target.title ? ` "${target.title.replace(/([\\"])/g, "\\$1")}"` : "";
  return `${target.kind === "image" ? "!" : ""}[${text}](${markdownDestination(path)}${title})`;
}
