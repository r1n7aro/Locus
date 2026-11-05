import type { InjectionKey } from "vue";
import { DocumentSessionCache } from "../../document/documentSessionCache";
import type { ProjectExplorerFilePreview } from "../../types/workbench";

/** Per-pane disk snapshots. Drafts stay owned by their live editor sessions. */
export class WorkspaceFilePreviewCache {
  private readonly entries = new DocumentSessionCache<ProjectExplorerFilePreview>({ capacity: 12 });

  get(key: string): ProjectExplorerFilePreview | undefined {
    return this.entries.get(key);
  }

  set(key: string, preview: ProjectExplorerFilePreview): void {
    // Bound retained text as well as entry count. Media and model previews are
    // never retained: their blob lease belongs to the live preview that releases it.
    if (preview.kind !== "text" || (preview.text?.length ?? 0) > 512 * 1024) {
      this.entries.delete(key);
      return;
    }
    this.entries.set(key, preview);
  }

  delete(key: string): void {
    this.entries.delete(key);
  }
}

export const WORKSPACE_FILE_PREVIEW_CACHE: InjectionKey<WorkspaceFilePreviewCache> = Symbol("workspaceFilePreviewCache");
