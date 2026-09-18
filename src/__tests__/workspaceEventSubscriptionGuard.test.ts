import { readdirSync, readFileSync } from "node:fs";
import { relative, resolve } from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    if (entry.name === "__tests__") return [];
    const path = resolve(directory, entry.name);
    return entry.isDirectory() ? sourceFiles(path) : /\.(ts|vue)$/.test(entry.name) ? [path] : [];
  });
}

describe("workspace event transport boundary", () => {
  it("allows only the window hub to own a native workspace listener and requires named consumers", () => {
    const violations: string[] = [];
    let nativeListeners = 0;
    for (const file of sourceFiles(resolve("src"))) {
      const path = relative(process.cwd(), file).replaceAll("\\", "/");
      const raw = readFileSync(file, "utf8");
      if (!/WORKSPACE_EVENT_NAME|locus:\/\/workspace-event|listenWorkspaceEvent/.test(raw)) continue;
      const source = file.endsWith(".vue")
        ? raw.slice(raw.indexOf(">", raw.indexOf("<script")) + 1, raw.indexOf("</script>"))
        : raw;
      const ast = ts.createSourceFile(`${file}.ts`, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
      const names = new Set(["WORKSPACE_EVENT_NAME"]);
      for (const statement of ast.statements) {
        if (!ts.isImportDeclaration(statement)) continue;
        const bindings = statement.importClause?.namedBindings;
        if (!bindings || !ts.isNamedImports(bindings)) continue;
        for (const element of bindings.elements) {
          if ((element.propertyName ?? element.name).text === "WORKSPACE_EVENT_NAME") names.add(element.name.text);
        }
      }
      function visit(node: ts.Node): void {
        if (ts.isCallExpression(node)) {
          const first = node.arguments[0];
          const workspaceEvent = first && (
            (ts.isIdentifier(first) && names.has(first.text))
            || (ts.isStringLiteral(first) && first.text === "locus://workspace-event")
          );
          if (workspaceEvent) {
            if (path === "src/services/workspaceEventHub.ts" && node.expression.getText(ast) === "listen") {
              nativeListeners += 1;
            } else {
              const expression = node.expression;
              const options = node.arguments[2];
              const namedOwner = options && ts.isObjectLiteralExpression(options)
                && options.properties.some((property) => ts.isPropertyAssignment(property)
                  && property.name.getText(ast) === "owner"
                  && ts.isStringLiteral(property.initializer) && property.initializer.text.length > 0);
              if (!ts.isPropertyAccessExpression(expression) || expression.name.text !== "subscribe" || !namedOwner) {
                violations.push(`${path}: ${node.expression.getText(ast)}`);
              }
            }
          }
        }
        ts.forEachChild(node, visit);
      }
      visit(ast);
    }
    expect(violations).toEqual([]);
    expect(nativeListeners).toBe(1);
  });
});
