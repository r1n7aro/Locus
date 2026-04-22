import { t } from "../../i18n";
import type {
  KnowledgeDocument,
  KnowledgeDocumentType,
  KnowledgeEditMode,
  KnowledgeDocumentPatch,
} from "../../types";

type EditModeDocument = Pick<
  KnowledgeDocument,
  "type" | "readOnly" | "aiMaintained" | "inheritAiConfig" | "externalSource"
>;

export function getKnowledgeEditMode(
  document: Pick<KnowledgeDocument, "readOnly" | "aiMaintained" | "inheritAiConfig"> | null | undefined,
): KnowledgeEditMode {
  if (document?.inheritAiConfig) return "inherit_parent";
  if (document?.readOnly) return "read_only";
  return document?.aiMaintained ? "auto" : "proposal";
}

export function buildKnowledgeEditModePatch(
  mode: KnowledgeEditMode,
): Pick<KnowledgeDocumentPatch, "readOnly" | "aiMaintained" | "inheritAiConfig" | "explicitMaintenanceRules"> {
  switch (mode) {
    case "inherit_parent":
      return { readOnly: false, inheritAiConfig: true };
    case "read_only":
      return { readOnly: true, inheritAiConfig: false, aiMaintained: false };
    case "auto":
      return { readOnly: false, inheritAiConfig: false, aiMaintained: true, explicitMaintenanceRules: true };
    default:
      return { readOnly: false, inheritAiConfig: false, aiMaintained: false };
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
  if (document?.readOnly) return true;
  const provider = document?.externalSource?.provider;
  return provider === "local_folder" || provider === "feishu";
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
    inheritInjectMode: true,
    summaryEnabled: defaultSummaryEnabledForType(type),
  };
}
