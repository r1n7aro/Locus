import { parser } from "@lezer/markdown";

export type KnowledgeDocumentOutlineItem = {
  id: string;
  level: number;
  text: string;
  from: number;
  to: number;
};

const ATX_HEADING_NAME = /^ATXHeading([1-6])$/;
const SETEXT_HEADING_NAME = /^SetextHeading([12])$/;

function plainHeadingText(rawHeading: string, isSetext: boolean): string {
  const headingLine = isSetext
    ? (rawHeading.split(/\r?\n/u, 1)[0] ?? "")
    : rawHeading
      .replace(/^ {0,3}#{1,6}(?:[\t ]+|$)/u, "")
      .replace(/[\t ]+#+[\t ]*$/u, "");

  return headingLine
    .replace(/!\[([^\]]*)\]\([^)]*\)/gu, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/gu, "$1")
    .replace(/<[^>]+>/gu, "")
    .replace(/[`*_~]/gu, "")
    .replace(/\\([\\`*{}\[\]()#+\-.!_>])/gu, "$1")
    .trim();
}

export function extractKnowledgeDocumentOutline(
  markdown: string,
): KnowledgeDocumentOutlineItem[] {
  if (!markdown.trim()) return [];

  const items: KnowledgeDocumentOutlineItem[] = [];
  const tree = parser.parse(markdown);
  tree.iterate({
    enter(node) {
      const atxMatch = ATX_HEADING_NAME.exec(node.name);
      const setextMatch = SETEXT_HEADING_NAME.exec(node.name);
      const level = Number(atxMatch?.[1] ?? setextMatch?.[1] ?? 0);
      if (!level) return;

      const text = plainHeadingText(
        markdown.slice(node.from, node.to),
        Boolean(setextMatch),
      );
      if (!text) return;

      items.push({
        id: `${level}:${node.from}`,
        level,
        text,
        from: node.from,
        to: node.to,
      });
    },
  });
  return items;
}
