const EXTERNAL_MARKDOWN_LINK_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

export function normalizeExternalMarkdownHref(rawHref: string | null | undefined): string | null {
  const href = rawHref?.trim();
  if (!href || href.startsWith("#")) return null;

  if (href.startsWith("//")) {
    return `https:${href}`;
  }

  try {
    const url = new URL(href);
    return EXTERNAL_MARKDOWN_LINK_PROTOCOLS.has(url.protocol) ? url.href : null;
  } catch {
    return null;
  }
}
