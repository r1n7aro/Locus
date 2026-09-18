import { searchWorkspaceEntries, type WorkspaceRef } from "../../../services/project";
import { searchWorkspaceAssets, searchWorkspaceSceneObjects } from "../../../services/asset";
import { knowledgeQuery } from "../../../services/knowledge";
import { viewTree } from "../../../services/view";
import type { MarkdownEditTarget } from "./markdownEditTarget";

export interface MarkdownResourceOption { path: string; name: string }

export async function searchMarkdownEditorResources(query: string, target: MarkdownEditTarget, workspaceRef: WorkspaceRef): Promise<MarkdownResourceOption[]> {
  const kind = target.reference?.kind;
  if (kind === "view") {
    const tree = await viewTree(workspaceRef);
    return tree.views.filter((view) => `${view.name} ${view.id}`.toLowerCase().includes(query.toLowerCase()))
      .slice(0, 30).map((view) => ({ path: view.id, name: view.name }));
  }
  if (kind === "knowledge") {
    const results = await knowledgeQuery({ query, limit: 30 }, workspaceRef);
    return results.filter((item) => (item.storageSource ?? "project") === "project")
      .map((item) => ({ path: item.path.startsWith(`${item.type}/`) ? item.path : `${item.type}/${item.path}`, name: item.title }));
  }
  const scene = target.url.match(/^(.*?\.unity)\//i)?.[1];
  if (scene && (kind === "unity-scene-object" || kind === "unity-property")) {
    const results = await searchWorkspaceSceneObjects(scene, query, 30, workspaceRef);
    return results.map((item) => ({ path: `${item.scenePath}/${item.objectPath}`, name: item.name }));
  }
  const jobs = await Promise.allSettled([
    searchWorkspaceEntries(query, workspaceRef, 50),
    searchWorkspaceAssets(query, ["Assets", "Packages", "ProjectSettings"], 50, workspaceRef),
  ]);
  const options: MarkdownResourceOption[] = [];
  if (jobs[0].status === "fulfilled") options.push(...jobs[0].value.filter((entry) => !entry.isDir).map((entry) => ({ path: entry.relPath, name: entry.name })));
  if (jobs[1].status === "fulfilled") options.push(...jobs[1].value.filter((entry) => !entry.isDirectory).map((entry) => ({ path: entry.path, name: entry.name })));
  if (jobs.every((job) => job.status === "rejected")) throw new Error("资源搜索失败");
  return [...new Map(options.filter((option) => {
    if (target.kind === "image") return /\.(?:png|jpg|jpeg|gif|webp|svg|bmp)$/i.test(option.path);
    return !kind?.startsWith("unity-") || /^(Assets|Packages)\//.test(option.path);
  }).map((option) => [option.path, option])).values()].slice(0, 30);
}
