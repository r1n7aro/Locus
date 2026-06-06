import type { ViewPackageFile } from "../../services/view";

interface MigratedViewRuntimeApiPattern {
  regex: RegExp;
  replacement: string;
}

const MIGRATED_VIEW_MEMBER_PATTERNS: MigratedViewRuntimeApiPattern[] = [
  {
    regex: /\bview\s*\.\s*(binding|serializedProperty|unitySerializedProperty)\b/g,
    replacement: "property",
  },
  {
    regex: /\bview\s*\.\s*(readBinding|discoverBinding|writeBinding|applyBindings)\b/g,
    replacement: "property.readProperty/property.write/property.apply",
  },
];

const MIGRATED_VIEW_RUNTIME_IMPORTS = new Map<string, string>([
  ["binding", "property"],
  ["serializedProperty", "property"],
  ["unitySerializedProperty", "property"],
  ["useUnityBinding", "property.fromPath/property.readProperty"],
]);

function sourcePosition(source: string, index: number): { line: number; column: number } {
  const before = source.slice(0, index);
  const lines = before.split(/\r?\n/);
  return {
    line: lines.length,
    column: lines[lines.length - 1].length + 1,
  };
}

function normalizeApiSnippet(value: string): string {
  return value.replace(/\s+/g, "");
}

function formatMigratedViewApiMessage(
  file: Pick<ViewPackageFile, "relPath">,
  source: string,
  index: number,
  api: string,
  replacement: string,
): string {
  const position = sourcePosition(source, index);
  return `${file.relPath}:${position.line}:${position.column} uses migrated View runtime API \`${api}\`; use \`${replacement}\`.`;
}

function findMigratedViewMemberUsage(file: Pick<ViewPackageFile, "relPath" | "content">): string | null {
  for (const pattern of MIGRATED_VIEW_MEMBER_PATTERNS) {
    pattern.regex.lastIndex = 0;
    const match = pattern.regex.exec(file.content);
    if (!match) continue;
    return formatMigratedViewApiMessage(
      file,
      file.content,
      match.index,
      normalizeApiSnippet(match[0]),
      pattern.replacement,
    );
  }
  return null;
}

function findMigratedViewRuntimeImport(file: Pick<ViewPackageFile, "relPath" | "content">): string | null {
  const source = file.content;
  const importPattern = /import\s*\{([\s\S]*?)\}\s*from\s*["']@locus\/view-runtime["']/g;
  for (const importMatch of source.matchAll(importPattern)) {
    const importBlock = importMatch[1];
    const importBlockOffset = (importMatch.index ?? 0) + importMatch[0].indexOf(importBlock);
    for (const [api, replacement] of MIGRATED_VIEW_RUNTIME_IMPORTS) {
      const specifierPattern = new RegExp(`\\b${api}\\b(?:\\s+as\\s+[A-Za-z_$][\\w$]*)?`);
      const specifierMatch = specifierPattern.exec(importBlock);
      if (!specifierMatch) continue;
      return formatMigratedViewApiMessage(
        file,
        source,
        importBlockOffset + specifierMatch.index,
        api,
        replacement,
      );
    }
  }
  return null;
}

export function findMigratedViewRuntimeApiUsage(
  file: Pick<ViewPackageFile, "relPath" | "content">,
): string | null {
  return findMigratedViewMemberUsage(file) ?? findMigratedViewRuntimeImport(file);
}
