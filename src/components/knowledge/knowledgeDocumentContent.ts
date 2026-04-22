export type KnowledgeDocumentEditableParts = {
  maintenanceRules: string;
  body: string;
  hasExplicitRulesSection: boolean;
  bodyHeading: string;
};

const DEFAULT_BODY_HEADING = "## Notes";

function stripMatchingHeading(content: string, title: string): string {
  const trimmed = content.trim();
  const lines = trimmed.split(/\r?\n/);
  if (lines[0]?.trim() === `# ${title}`) {
    return lines.slice(1).join("\n").trim();
  }
  return trimmed;
}

function parseMarkdownH2Heading(line: string): { normalized: string; raw: string } | null {
  const trimmed = line.trim();
  if (!trimmed.startsWith("## ") || trimmed.startsWith("###")) return null;
  return {
    normalized: trimmed.slice(3).trim().toLowerCase(),
    raw: trimmed,
  };
}

function isMaintenanceRulesHeading(heading: string): boolean {
  return ["maintenance rules", "rules", "维护规则", "维护说明"].includes(heading.trim());
}

function isDefaultBodyHeading(heading: string): boolean {
  return [
    "notes",
    "content",
    "memory content",
    "笔记",
    "内容",
    "实际内容",
  ].includes(heading.trim());
}

export function splitKnowledgeDocumentContent(
  content: string,
  title: string,
): KnowledgeDocumentEditableParts {
  const stripped = stripMatchingHeading(content, title);
  if (!stripped) {
    return {
      maintenanceRules: "",
      body: "",
      hasExplicitRulesSection: false,
      bodyHeading: DEFAULT_BODY_HEADING,
    };
  }

  const rulesLines: string[] = [];
  const bodyLines: string[] = [];
  let inRulesSection = false;
  let hasExplicitRulesSection = false;
  let bodyHeading = DEFAULT_BODY_HEADING;

  for (const line of stripped.split(/\r?\n/)) {
    const heading = parseMarkdownH2Heading(line);
    if (heading) {
      if (isMaintenanceRulesHeading(heading.normalized)) {
        inRulesSection = true;
        hasExplicitRulesSection = true;
        continue;
      }

      if (
        inRulesSection
        && bodyLines.length === 0
        && isDefaultBodyHeading(heading.normalized)
      ) {
        inRulesSection = false;
        bodyHeading = heading.raw;
        continue;
      }

      if (inRulesSection) {
        inRulesSection = false;
      }
    }

    if (inRulesSection) rulesLines.push(line);
    else bodyLines.push(line);
  }

  const maintenanceRules = rulesLines.join("\n").trim();
  const body = bodyLines.join("\n").trim();

  if (!hasExplicitRulesSection) {
    return {
      maintenanceRules: body,
      body: "",
      hasExplicitRulesSection: false,
      bodyHeading,
    };
  }

  return {
    maintenanceRules,
    body,
    hasExplicitRulesSection: true,
    bodyHeading,
  };
}

export function buildKnowledgeDocumentContent(params: {
  title: string;
  maintenanceRules: string;
  body: string;
  bodyHeading?: string;
}): string {
  const title = params.title.trim() || "Memory";
  const maintenanceRules = params.maintenanceRules.trim();
  const body = params.body.trim();
  const bodyHeading = params.bodyHeading?.trim() || DEFAULT_BODY_HEADING;

  const sections = [
    `# ${title}`,
    maintenanceRules ? `## Maintenance Rules\n${maintenanceRules}` : "## Maintenance Rules",
  ];

  if (!body) {
    sections.push(bodyHeading);
  } else if (/^#{2,6}\s+/.test(body)) {
    sections.push(body);
  } else {
    sections.push(`${bodyHeading}\n${body}`);
  }

  return `${sections.join("\n\n").trimEnd()}\n`;
}
