import type { Text } from "@codemirror/state";
import type {
  KnowledgeDirectoryConfig,
  KnowledgeDirectoryConfigRecord,
  KnowledgeDocument,
  KnowledgeDocumentSection,
} from "../../types";
import type { WorkspaceRef } from "../../services/project";
import type { KnowledgeTextConflict } from "./knowledgeCollaborativeEditing";
import { createKnowledgeEditorDraftValues } from "./knowledgeEditorDrafts";
import { KnowledgeEditorSessionCache } from "./knowledgeEditorSessionCache";
import { MarkdownEditorSessionCache } from "../ui/markdown-editor/markdownEditorSessionCache";

export type KnowledgeEditorDraftValues = ReturnType<typeof createKnowledgeEditorDraftValues>;
export type KnowledgeSectionConflicts = Record<
  KnowledgeDocumentSection,
  KnowledgeTextConflict[]
>;
export type DirectoryMarkdownField = "summary" | "maintenanceRules";

export interface KnowledgeDocumentEditorSession {
  drafts: KnowledgeEditorDraftValues;
  baseDrafts: KnowledgeEditorDraftValues;
  conflictResolutionDrafts: KnowledgeEditorDraftValues;
  dirtySections: Set<KnowledgeDocumentSection>;
  sectionConflicts: KnowledgeSectionConflicts;
  saveBlockedSections: Set<KnowledgeDocumentSection>;
  sectionTextBuffers: Map<KnowledgeDocumentSection, Text>;
  baseSectionTexts: Map<KnowledgeDocumentSection, Text>;
  fileNameDraft: string;
  fileNameDirty: boolean;
  saveRevision: number;
}

export interface KnowledgeDirectoryEditorSession {
  draft: KnowledgeDirectoryConfig;
  baseDraft: KnowledgeDirectoryConfig;
  markdownTextBuffers: Map<DirectoryMarkdownField, Text>;
  markdownBaseTexts: Map<DirectoryMarkdownField, Text>;
  markdownDirtyFields: Set<DirectoryMarkdownField>;
  submittedDraft: KnowledgeDirectoryConfig | null;
  dirty: boolean;
}

function workspaceScopeKey(workspaceRef: WorkspaceRef | null | undefined): string {
  if (!workspaceRef) return "workspace:unbound";
  return JSON.stringify([
    "workspace",
    workspaceRef.checkoutId,
    workspaceRef.expectedGeneration ?? "current",
  ]);
}

export function knowledgeDocumentEditorSessionKey(
  workspaceRef: WorkspaceRef | null | undefined,
  document: KnowledgeDocument | null | undefined,
): string {
  if (!document) return "";
  return JSON.stringify([
    workspaceScopeKey(workspaceRef),
    "document",
    document.type,
    document.id || document.path,
  ]);
}

export function knowledgeDirectoryEditorSessionKey(
  workspaceRef: WorkspaceRef | null | undefined,
  directory: KnowledgeDirectoryConfigRecord | null | undefined,
): string {
  if (!directory) return "";
  return JSON.stringify([
    workspaceScopeKey(workspaceRef),
    "directory",
    directory.type,
    directory.path,
  ]);
}

export class KnowledgeEditorWorkspaceSessionStore {
  readonly documents: KnowledgeEditorSessionCache<KnowledgeDocumentEditorSession>;
  readonly directories: KnowledgeEditorSessionCache<KnowledgeDirectoryEditorSession>;
  readonly markdownEditors: MarkdownEditorSessionCache;

  constructor(
    documentCapacity = 24,
    directoryCapacity = 24,
    markdownEditorCapacity = 96,
  ) {
    this.documents = new KnowledgeEditorSessionCache(
      documentCapacity,
      (session) =>
        session.dirtySections.size === 0
        && session.saveBlockedSections.size === 0
        && !session.fileNameDirty
        && !Object.values(session.sectionConflicts).some((items) => items.length > 0),
    );
    this.directories = new KnowledgeEditorSessionCache(
      directoryCapacity,
      (session) => !session.dirty,
    );
    this.markdownEditors = new MarkdownEditorSessionCache(markdownEditorCapacity);
  }
}
