import * as ts from "typescript";

export type ViewCompileDiagnosticCategory = "error" | "warning" | "suggestion" | "message";

export interface ViewCompileDiagnostic {
  category: ViewCompileDiagnosticCategory;
  code: number;
  message: string;
  fileName?: string;
  line?: number;
  column?: number;
}

export interface ViewTranspileResult {
  code: string;
  diagnostics: ViewCompileDiagnostic[];
}

export interface TransformResult {
  code: string;
  introducedNames: string[];
  diagnostics: ViewCompileDiagnostic[];
}

export interface ViewSfcCompileResult {
  code: string;
  styles: string[];
  scopeId: string | null;
  diagnostics: ViewCompileDiagnostic[];
}

export class ViewCompileError extends Error {
  diagnostics: ViewCompileDiagnostic[];

  constructor(diagnostics: ViewCompileDiagnostic[]) {
    super(formatViewCompileDiagnostics(diagnostics));
    this.name = "ViewCompileError";
    this.diagnostics = diagnostics;
  }
}

const VIEW_TS_COMPILER_OPTIONS: ts.CompilerOptions = {
  target: ts.ScriptTarget.ES2020,
  module: ts.ModuleKind.ESNext,
  isolatedModules: true,
  verbatimModuleSyntax: true,
  jsx: ts.JsxEmit.Preserve,
  useDefineForClassFields: false,
  sourceMap: false,
  inlineSourceMap: false,
  inlineSources: false,
  removeComments: false,
};

function viewCompilerOptions(moduleKind: ts.ModuleKind): ts.CompilerOptions {
  return {
    ...VIEW_TS_COMPILER_OPTIONS,
    module: moduleKind,
  };
}

function diagnosticCategory(category: ts.DiagnosticCategory): ViewCompileDiagnosticCategory {
  switch (category) {
    case ts.DiagnosticCategory.Error:
      return "error";
    case ts.DiagnosticCategory.Warning:
      return "warning";
    case ts.DiagnosticCategory.Suggestion:
      return "suggestion";
    case ts.DiagnosticCategory.Message:
    default:
      return "message";
  }
}

function toViewDiagnostic(diagnostic: ts.Diagnostic): ViewCompileDiagnostic {
  const location = diagnostic.file && typeof diagnostic.start === "number"
    ? diagnostic.file.getLineAndCharacterOfPosition(diagnostic.start)
    : null;
  return {
    category: diagnosticCategory(diagnostic.category),
    code: diagnostic.code,
    message: ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n"),
    fileName: diagnostic.file?.fileName,
    line: location ? location.line + 1 : undefined,
    column: location ? location.character + 1 : undefined,
  };
}

export function sfcErrorCode(error: unknown): number {
  const code = (error as { code?: unknown }).code;
  return typeof code === "number" ? code : 0;
}

export function toSfcDiagnostic(error: unknown, fileName: string): ViewCompileDiagnostic {
  const loc = (error as { loc?: { start?: { line?: number; column?: number } } }).loc?.start;
  const message = error instanceof Error ? error.message : String(error);
  return {
    category: "error",
    code: sfcErrorCode(error),
    message,
    fileName,
    line: loc?.line,
    column: loc?.column,
  };
}

export function throwOnSfcErrors(errors: readonly unknown[], fileName: string) {
  if (!errors.length) return;
  throw new ViewCompileError(errors.map((error) => toSfcDiagnostic(error, fileName)));
}

function formatDiagnosticLocation(diagnostic: ViewCompileDiagnostic): string {
  const location = diagnostic.fileName
    ? [
        diagnostic.fileName,
        diagnostic.line,
        diagnostic.column,
      ].filter((part) => part !== undefined && part !== "").join(":")
    : "view";
  return `${location} TS${diagnostic.code}`;
}

export function formatViewCompileDiagnostics(diagnostics: ViewCompileDiagnostic[]): string {
  if (!diagnostics.length) return "View compile failed.";
  return [
    "View compile failed.",
    ...diagnostics.map((diagnostic) =>
      `${formatDiagnosticLocation(diagnostic)}: ${diagnostic.message}`,
    ),
  ].join("\n");
}

export function transpileViewTypeScript(
  source: string,
  fileName = "view.ts",
): ViewTranspileResult {
  return transpileViewTypeScriptWithOptions(
    source,
    fileName,
    viewCompilerOptions(ts.ModuleKind.ESNext),
  );
}

function transpileViewTypeScriptWithOptions(
  source: string,
  fileName: string,
  compilerOptions: ts.CompilerOptions,
): ViewTranspileResult {
  const result = ts.transpileModule(source, {
    compilerOptions,
    fileName,
    reportDiagnostics: true,
  });
  const diagnostics = (result.diagnostics ?? []).map(toViewDiagnostic);
  const errors = diagnostics.filter((diagnostic) => diagnostic.category === "error");
  if (errors.length) {
    throw new ViewCompileError(errors);
  }
  return {
    code: result.outputText,
    diagnostics,
  };
}

export function extractVueScriptSetup(source: string): string {
  const match = source.match(/<script\b[^>]*\bsetup\b[^>]*>([\s\S]*?)<\/script>/i);
  return match?.[1]?.trim() || "";
}

export function extractVueScript(source: string): string {
  const match = source.match(/<script\b(?![^>]*\bsetup\b)[^>]*>([\s\S]*?)<\/script>/i);
  return match?.[1]?.trim() || "";
}

export function sanitizeTemplateExpressions(template: string): string {
  return template.replace(/\s+as\s+[A-Za-z_$][\w$.[\]<>, |&?]*/g, "");
}

function parseImportBindings(importClause: string, moduleVariable: string): { code: string; names: string[] } {
  const names: string[] = [];
  const parts: string[] = [];
  const trimmed = importClause.trim();
  const namespaceMatch = trimmed.match(/^\*\s+as\s+([A-Za-z_$][\w$]*)$/);
  if (namespaceMatch) {
    names.push(namespaceMatch[1]);
    return { code: `const ${namespaceMatch[1]} = ${moduleVariable};`, names };
  }

  const namedMatch = trimmed.match(/{([\s\S]*?)}/);
  const defaultPart = trimmed.replace(/{[\s\S]*?}/, "").replace(/,$/, "").trim();
  if (defaultPart) {
    const defaultName = defaultPart.split(",")[0]?.trim();
    if (defaultName) {
      parts.push(`const ${defaultName} = ${moduleVariable}.default ?? ${moduleVariable};`);
      names.push(defaultName);
    }
  }

  if (namedMatch) {
    const bindings = namedMatch[1]
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean)
      .map((item) => {
        const [source, alias] = item.split(/\s+as\s+/).map((part) => part.trim());
        const local = alias || source;
        names.push(local);
        return alias ? `${source}: ${alias}` : source;
      })
      .join(", ");
    parts.push(`const { ${bindings} } = ${moduleVariable};`);
  }

  return { code: parts.join("\n"), names };
}

function transformImports(source: string): TransformResult {
  const introducedNames: string[] = [];
  let moduleIndex = 0;
  let code = source.replace(/^\s*import\s+type\s+[\s\S]*?;\s*$/gm, "");
  code = code.replace(/import\s+([\s\S]*?)\s+from\s+["']([^"']+)["'];?/g, (_full, clause, specifier) => {
    const moduleVariable = `__module${moduleIndex}`;
    moduleIndex += 1;
    const parsed = parseImportBindings(clause, moduleVariable);
    introducedNames.push(...parsed.names);
    return `const ${moduleVariable} = __import(${JSON.stringify(specifier)});\n${parsed.code}`;
  });
  code = code.replace(/import\s+["']([^"']+)["'];?/g, (_full, specifier) => {
    return `__import(${JSON.stringify(specifier)});`;
  });
  return { code, introducedNames, diagnostics: [] };
}

function collectTopLevelNames(code: string): string[] {
  const names = new Set<string>();
  const patterns = [
    /\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)/g,
    /\b(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*\(/g,
    /\bclass\s+([A-Za-z_$][\w$]*)\b/g,
  ];
  for (const pattern of patterns) {
    let match = pattern.exec(code);
    while (match) {
      names.add(match[1]);
      match = pattern.exec(code);
    }
  }
  return Array.from(names);
}

function isPublicSetupBinding(name: string): boolean {
  return !name.startsWith("_") && !name.startsWith("$");
}

export function transformViewScriptSetup(source: string, fileName = "src/App.vue"): TransformResult {
  const transpiled = transpileViewTypeScript(source, `${fileName}?script-setup.ts`);
  const imports = transformImports(transpiled.code);
  return {
    code: imports.code,
    introducedNames: Array.from(
      new Set([...imports.introducedNames, ...collectTopLevelNames(imports.code)]),
    ).filter(isPublicSetupBinding),
    diagnostics: transpiled.diagnostics,
  };
}

export function compileViewModule(source: string, fileName = "view.ts"): ViewTranspileResult {
  const transpiled = transpileViewTypeScriptWithOptions(
    source,
    fileName,
    viewCompilerOptions(ts.ModuleKind.CommonJS),
  );
  return {
    code: `const require = __import;\n${transpiled.code}`,
    diagnostics: transpiled.diagnostics,
  };
}

export function transformModuleSource(source: string, fileName = "view.ts"): string {
  return compileViewModule(source, fileName).code;
}
