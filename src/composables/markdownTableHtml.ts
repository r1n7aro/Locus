const TABLE_WRAPPER_CLASS = "md-table-wrap";
const WRAPPED_TABLE_RE = /<div class="md-table-wrap">\s*(<table\b[\s\S]*?<\/table>)\s*<\/div>/gi;
const TABLE_RE = /<table\b[\s\S]*?<\/table>/gi;

export function wrapMarkdownTables(html: string): string {
  if (!/<table\b/i.test(html)) return html;

  const normalizedHtml = html.replace(WRAPPED_TABLE_RE, "$1");
  return normalizedHtml.replace(TABLE_RE, (tableHtml) => (
    `<div class="${TABLE_WRAPPER_CLASS}">${tableHtml}</div>`
  ));
}
