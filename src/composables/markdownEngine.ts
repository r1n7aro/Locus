import { Marked } from "marked";
import { markedHighlight } from "marked-highlight";
import hljs from "../hljs";
import { renderHighlightedCodeLines } from "./markdownCodeLines";
import {
  isMarkdownUnityObjectFenceLanguage,
  isMarkdownUnityPropertyFenceLanguage,
} from "./markdownInject";
import { wrapMarkdownTables } from "./markdownTableHtml";

export function escapeMarkdownHtml(source: string): string {
  return source
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/**
 * Shared Marked instance for every markdown surface. Construction registers
 * the highlight extension and hooks, so building one per component instance
 * (the old `<script setup>` const) multiplied that cost by the number of
 * rendered blocks; parsing itself is stateless and safe to share.
 */
export const markdownEngine = new Marked(
  markedHighlight({
    langPrefix: "hljs language-",
    highlight(code: string, lang: string) {
      const normalizedLang = lang.trim().toLowerCase();
      if (
        isMarkdownUnityObjectFenceLanguage(normalizedLang)
        || isMarkdownUnityPropertyFenceLanguage(normalizedLang)
      ) {
        return escapeMarkdownHtml(code);
      }
      if (normalizedLang === "tree") {
        return renderHighlightedCodeLines(escapeMarkdownHtml(code), false);
      }

      let highlighted = escapeMarkdownHtml(code);
      if (normalizedLang && hljs.getLanguage(normalizedLang)) {
        highlighted = hljs.highlight(code, { language: normalizedLang }).value;
      }
      return renderHighlightedCodeLines(highlighted);
    },
  }),
  {
    breaks: true,
    gfm: true,
    hooks: {
      postprocess(html) {
        return wrapMarkdownTables(html);
      },
    },
  },
);
