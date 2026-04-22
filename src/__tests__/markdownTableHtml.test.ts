import { describe, expect, it } from "vitest";
import { wrapMarkdownTables } from "../composables/markdownTableHtml";

describe("markdownTableHtml", () => {
  it("wraps table markup in a dedicated scroll container", () => {
    const html = [
      "<p>before</p>",
      "<table><thead><tr><th>A</th></tr></thead><tbody><tr><td>B</td></tr></tbody></table>",
      "<p>after</p>",
    ].join("");

    expect(wrapMarkdownTables(html)).toBe([
      "<p>before</p>",
      '<div class="md-table-wrap"><table><thead><tr><th>A</th></tr></thead><tbody><tr><td>B</td></tr></tbody></table></div>',
      "<p>after</p>",
    ].join(""));
  });

  it("keeps an already wrapped table at a single wrapper level", () => {
    const html = '<div class="md-table-wrap"><table><tbody><tr><td>A</td></tr></tbody></table></div>';
    const wrapped = wrapMarkdownTables(html);

    expect(wrapped).toBe(html);
    expect(wrapped.match(/md-table-wrap/g)).toHaveLength(1);
  });
});
