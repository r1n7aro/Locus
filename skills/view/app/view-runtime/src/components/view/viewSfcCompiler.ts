import {
  compileScript as compileVueScript,
  compileStyle as compileVueStyle,
  compileTemplate as compileVueTemplate,
  parse as parseVueSfc,
  type SFCDescriptor,
} from "@vue/compiler-sfc";
import {
  type ViewCompileDiagnostic,
  ViewCompileError,
  type ViewSfcCompileResult,
  throwOnSfcErrors,
  toSfcDiagnostic,
  transformModuleSource,
} from "./viewCompiler";

export { transformModuleSource };

function parseViewSfc(source: string, fileName: string): SFCDescriptor {
  const parsed = parseVueSfc(source, { filename: fileName, sourceMap: false });
  throwOnSfcErrors(parsed.errors, fileName);
  return parsed.descriptor;
}

function viewSfcScopeId(source: string, fileName: string): string {
  let hash = 2166136261;
  const input = `${fileName}\n${source}`;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return `data-v-${(hash >>> 0).toString(36)}`;
}

function compileSfcStyles(
  descriptor: SFCDescriptor,
  scopeId: string,
  fileName: string,
): { styles: string[]; diagnostics: ViewCompileDiagnostic[] } {
  const styles: string[] = [];
  const diagnostics: ViewCompileDiagnostic[] = [];

  descriptor.styles.forEach((style, index) => {
    const result = compileVueStyle({
      source: style.content,
      filename: `${fileName}?style=${index}`,
      id: scopeId,
      scoped: style.scoped,
      preprocessLang: style.lang as never,
    });
    const styleDiagnostics = result.errors.map((error) =>
      toSfcDiagnostic(error, `${fileName}?style=${index}`),
    );
    diagnostics.push(...styleDiagnostics);
    if (!styleDiagnostics.some((diagnostic) => diagnostic.category === "error")) {
      styles.push(result.code);
    }
  });

  const errors = diagnostics.filter((diagnostic) => diagnostic.category === "error");
  if (errors.length) {
    throw new ViewCompileError(errors);
  }

  return { styles, diagnostics };
}

export function compileViewSfc(source: string, fileName = "src/App.vue"): ViewSfcCompileResult {
  const descriptor = parseViewSfc(source, fileName);
  const hasScopedStyle = descriptor.styles.some((style) => style.scoped);
  const scopeId = viewSfcScopeId(source, fileName);
  const publicScopeId = hasScopedStyle ? scopeId : null;
  const diagnostics: ViewCompileDiagnostic[] = [];

  const script = descriptor.script || descriptor.scriptSetup
    ? compileVueScript(descriptor, {
        id: scopeId,
        genDefaultAs: "__sfc_main",
        inlineTemplate: false,
      })
    : null;

  const templateResult = descriptor.template
    ? compileVueTemplate({
        source: descriptor.template.content,
        filename: fileName,
        id: scopeId,
        scoped: hasScopedStyle,
        slotted: descriptor.slotted,
        transformAssetUrls: false,
        compilerOptions: {
          bindingMetadata: script?.bindings ?? {},
          expressionPlugins: ["typescript"],
        },
      })
    : null;

  if (templateResult) {
    throwOnSfcErrors(templateResult.errors, fileName);
    diagnostics.push(
      ...(templateResult.tips ?? []).map((tip): ViewCompileDiagnostic => ({
        category: "warning",
        code: 0,
        message: String(tip),
        fileName,
      })),
    );
  }

  const styleResult = compileSfcStyles(descriptor, scopeId, fileName);
  diagnostics.push(...styleResult.diagnostics);

  const scriptCode = script?.content ?? "const __sfc_main = {};";
  const renderCode = templateResult?.code ?? "function render() { return null; }";
  const scopeCode = publicScopeId
    ? `\n__sfc_main.__scopeId = ${JSON.stringify(publicScopeId)};`
    : "";
  const code = [
    scriptCode,
    renderCode,
    "__sfc_main.render = render;",
    scopeCode,
    "export default __sfc_main;",
  ].join("\n");

  return {
    code: transformModuleSource(code, fileName),
    styles: styleResult.styles,
    scopeId: publicScopeId,
    diagnostics,
  };
}
