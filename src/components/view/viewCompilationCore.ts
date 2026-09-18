import { compileStyle } from "@vue/compiler-sfc";
import type { Rule } from "postcss";
import selectorParser from "postcss-selector-parser";
import { compileViewSfc, transformModuleSource } from "./viewSfcCompiler";
import { findMigratedViewRuntimeApiUsage } from "./viewPackageDiagnostics";
import { viewFileContent } from "./viewPackageFiles";
import { viewSourceHash, type CompiledViewPackage, type ViewCompileRequest } from "./viewCompilationTypes";
import type { ViewSfcCompileResult } from "./viewCompiler";

const moduleCache = new Map<string, { source: string; result: ViewSfcCompileResult; size: number }>();
let cachedBytes = 0;
function compileCached(key: string, source: string, compile: () => ViewSfcCompileResult): ViewSfcCompileResult {
  const cacheKey = `${key}:${viewSourceHash(source)}`;
  const cached = moduleCache.get(cacheKey);
  if (cached?.source === source) { moduleCache.delete(cacheKey); moduleCache.set(cacheKey, cached); return cached.result; }
  const result = compile();
  const size = 2 * (source.length + result.code.length + result.styles.join("").length);
  if (cached) cachedBytes -= cached.size;
  moduleCache.set(cacheKey, { source, result, size }); cachedBytes += size;
  while (moduleCache.size > 96 || cachedBytes > 32 * 1024 * 1024) {
    const first = moduleCache.keys().next().value!;
    cachedBytes -= moduleCache.get(first)!.size; moduleCache.delete(first);
  }
  return result;
}

/** Package CSS stays in its component subtree, including legacy html/body rules. */
export function scopeViewCss(css: string, scopeId: string): string {
  const root = `:where([data-locus-view-scope="${scopeId}"])`;
  const boundary = selectorParser().astSync(root).first.first!;
  const result = compileStyle({
    source: css, filename: "view.css", id: scopeId,
    postcssPlugins: [{
      postcssPlugin: "locus-view-boundary",
      Rule(rule: Rule) {
        if (rule.parent?.type === "atrule" && /keyframes$/i.test(rule.parent.name)) return;
        rule.selector = selectorParser((selectors) => {
          selectors.each((selector) => {
            const isDocumentRoot = (node: selectorParser.Node | undefined) => node?.type === "tag" && ["html", "body"].includes(node.value.toLowerCase()) || node?.type === "pseudo" && node.value === ":root";
            const startsAtRoot = isDocumentRoot(selector.first) || selector.first?.toString() === root;
            selector.walkTags((node) => { if (isDocumentRoot(node)) node.replaceWith(boundary.clone()); });
            selector.walkPseudos((node) => { if (isDocumentRoot(node)) node.replaceWith(boundary.clone()); });
            if (startsAtRoot) {
              while (selector.nodes[1]?.type === "combinator" && ["", ">"].includes(selector.nodes[1].value.trim()) && selector.nodes[2]?.toString() === root) {
                selector.nodes[2]!.remove(); selector.nodes[1]!.remove();
              }
            } else {
              selector.prepend(selectorParser.combinator({ value: " " }));
              selector.prepend(boundary.clone());
            }
          });
        }).processSync(rule.selector);
      },
    }],
  });
  if (result.errors.length) throw new Error(result.errors.map(String).join("\n"));
  return result.code;
}

export function compileViewPackageSource(request: ViewCompileRequest): CompiledViewPackage {
  const { detail, scopeId, key } = request;
  const modules: CompiledViewPackage["modules"] = {};
  for (const file of detail.files) {
    if (file.truncated) throw new Error(`${file.relPath}: View source is truncated.`);
    if (!/\.(vue|ts|js|css|json)$/i.test(file.relPath) || /\.d\.ts$/i.test(file.relPath)) continue;
    if (/\.(vue|ts|js)$/i.test(file.relPath)) {
      const migration = findMigratedViewRuntimeApiUsage(file);
      if (migration) throw new Error(migration);
    }
    modules[file.relPath] = compileCached(`${scopeId}/${file.relPath}`, file.content, () => {
    if (file.relPath.endsWith(".vue")) {
      const result = compileViewSfc(file.content, `${scopeId}/${file.relPath}`);
      return { ...result, styles: result.styles.map((css) => scopeViewCss(css, scopeId)) };
    } else if (file.relPath.endsWith(".css")) {
      return { code: "", styles: [scopeViewCss(file.content, scopeId)], scopeId: null, diagnostics: [] };
    } else {
      const code = file.relPath.endsWith(".json")
        ? `module.exports = ${JSON.stringify(JSON.parse(file.content))};`
        : transformModuleSource(file.content, file.relPath);
      return { code, styles: [], scopeId: null, diagnostics: [] };
    }
    });
  }
  return {
    key, scopeId, modules,
    styles: detail.manifest.style ? [scopeViewCss(viewFileContent(detail, detail.manifest.style), scopeId)] : [],
    scriptKey: viewSourceHash(JSON.stringify([detail.manifest, Object.entries(modules).map(([path, module]) => [path, module.code])])),
  };
}
