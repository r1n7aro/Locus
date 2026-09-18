import { t } from "../../i18n";
import type { WorkspaceCheckoutDescriptor } from "../../services/project";
import type { DevelopmentResourceRef, ProjectKnowledgeDocument } from "../../types/workbench";

interface WorkbenchKnowledgeCatalog {
  documents: () => readonly ProjectKnowledgeDocument[];
  refresh: () => Promise<void>;
}

function normalizePath(path: string): string {
  const segments: string[] = [];
  for (const segment of path.trim().replace(/\\/g, "/").split("/")) {
    if (segment === ".") continue;
    if (segment === ".." && segments.length > 0 && segments[segments.length - 1] !== "..") {
      segments.pop();
    } else {
      segments.push(segment);
    }
  }
  return segments.join("/");
}

export async function resolveWorkbenchFileTarget(
  filePath: string,
  checkout: Pick<WorkspaceCheckoutDescriptor, "checkoutId" | "projectId" | "root">,
  catalog: WorkbenchKnowledgeCatalog,
): Promise<DevelopmentResourceRef> {
  const requestedPath = normalizePath(filePath);
  const rootPrefix = `${normalizePath(checkout.root).replace(/\/+$/, "")}/`;
  const path = requestedPath.toLocaleLowerCase().startsWith(rootPrefix.toLocaleLowerCase())
    ? requestedPath.slice(rootPrefix.length)
    : requestedPath;
  const match = path.match(/^Locus\/knowledge\/(design|plan|memory|skill|reference)\/(.+\.(?:md|csv))$/i);
  if (!match) return { kind: "workspaceFile", projectId: checkout.projectId, path };

  const docType = match[1]!.toLocaleLowerCase();
  const documentPath = match[2]!.toLocaleLowerCase();
  const findDocument = () => catalog.documents().find((document) => {
    if (document.type !== docType) return false;
    if (document.sourceCheckoutId !== checkout.checkoutId
      && !document.availableCheckoutIds.includes(checkout.checkoutId)) return false;
    const candidatePath = normalizePath(document.path).replace(/^\/+|\/+$/g, "").toLocaleLowerCase();
    return candidatePath === documentPath || candidatePath === `${docType}/${documentPath}`;
  });
  let document = findDocument();
  if (!document) {
    // A write action can be opened before the knowledge watcher refreshes the catalog.
    await catalog.refresh();
    document = findDocument();
  }
  if (!document) throw new Error(t("workbench.unavailable.knowledge"));
  return { kind: "knowledge", projectId: checkout.projectId, documentId: document.id };
}
