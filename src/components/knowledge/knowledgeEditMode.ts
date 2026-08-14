import { t } from "../../i18n";
import type {
  KnowledgeDocument,
  KnowledgeDocumentType,
  KnowledgeEditMode,
  KnowledgeDocumentPatch,
} from "../../types";

type EditModeDocument = Pick<
  KnowledgeDocument,
  "type" | "readOnly" | "aiEditMode" | "aiMaintained" | "storageSource" | "externalSource"
>;

export function getKnowledgeEditMode(
  document: Pick<KnowledgeDocument, "aiEditMode" | "aiMaintained"> | null | undefined,
): KnowledgeEditMode {
  const mode = document?.aiEditMode
    ?? (document?.aiMaintained === "inherit"
      ? "inherit"
      : document?.aiMaintained
        ? "auto"
        : "confirm");
  switch (mode) {
    case "inherit":
      return "inherit_parent";
    case "auto":
      return "auto";
    case "confirm":
      return "proposal";
    default:
      return "disabled";
  }
}

export function buildKnowledgeEditModePatch(
  mode: KnowledgeEditMode,
): Pick<KnowledgeDocumentPatch, "aiEditMode"> {
  switch (mode) {
    case "inherit_parent":
      return { aiEditMode: "inherit" };
    case "auto":
      return { aiEditMode: "auto" };
    case "disabled":
      return { aiEditMode: "disabled" };
    case "proposal":
      return { aiEditMode: "confirm" };
    default:
      return { aiEditMode: "disabled" };
  }
}

export function defaultKnowledgeEditMode(_type: KnowledgeDocumentType): KnowledgeEditMode {
  return "inherit_parent";
}

export function defaultExplicitMaintenanceRulesForType(type: KnowledgeDocumentType): boolean {
  return type === "memory";
}

export function defaultSummaryEnabledForType(type: KnowledgeDocumentType): boolean {
  return type === "reference" || type === "skill";
}

export function isKnowledgeEditModeLocked(document: EditModeDocument | null | undefined): boolean {
  if (document?.readOnly || document?.storageSource === "app") return true;
  const provider = document?.externalSource?.provider;
  return provider === "local_folder" || provider === "feishu" || provider === "package";
}

export function defaultMaintenanceRulesForType(type: KnowledgeDocumentType): string | null {
  switch (type) {
    case "design":
      return t("knowledge.defaults.rules.design");
    case "memory":
      return t("knowledge.defaults.rules.memory");
    case "skill":
      return t("knowledge.defaults.rules.skill");
    case "reference":
      return t("knowledge.defaults.rules.reference");
    default:
      return null;
  }
}

export function buildKnowledgeCreateDefaults(type: KnowledgeDocumentType) {
  const mode = defaultKnowledgeEditMode(type);
  const patch = buildKnowledgeEditModePatch(mode);
  return {
    ...patch,
    injectMode: "inherit" as const,
  };
}
