import { walkHtmlText } from "./markdownInject";

const IMAGE_EXT_RE = /\.(?:png|jpe?g|gif|bmp|webp|svg)(?:[?#][^\s<>"']*)?$/i;
const WEB_IMAGE_SOURCE_RE = /^(?:https?:)?\/\//i;
const DIRECT_IMAGE_SOURCE_RE = /^(?:https?:)?\/\/|^data:image\/|^blob:|^http:\/\/locus-binary\.localhost\//i;
const LOCAL_IMAGE_SOURCE_RE = /^(?:[A-Za-z]:[\\/]|\\\\|file:\/\/|\/(?!\/)|(?:Assets|Packages|ProjectSettings|src|src-tauri|docs|public)\/)/i;

function escapeAttr(source: string): string {
  return source
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function decodeHtml(source: string): string {
  return source
    .replace(/&quot;/g, "\"")
    .replace(/&#39;/g, "'")
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

function attrValue(tag: string, attr: string): string {
  const match = tag.match(new RegExp(`\\s${attr}\\s*=\\s*("([^"]*)"|'([^']*)'|([^\\s"'>]+))`, "i"));
  return decodeHtml(match?.[2] ?? match?.[3] ?? match?.[4] ?? "");
}

function trimStandaloneImageSource(source: string): string {
  const trimmed = source.trim();
  const quoted = trimmed.match(/^["'](.+)["']$/);
  return quoted ? quoted[1].trim() : trimmed;
}

export function isMarkdownImageSource(source: string): boolean {
  const normalized = trimStandaloneImageSource(source);
  if (!normalized) return false;
  if (/^data:image\//i.test(normalized) || /^blob:/i.test(normalized)) return true;
  if (!IMAGE_EXT_RE.test(normalized)) return false;
  return WEB_IMAGE_SOURCE_RE.test(normalized) || LOCAL_IMAGE_SOURCE_RE.test(normalized);
}

export function shouldResolveMarkdownImageSource(source: string): boolean {
  const normalized = trimStandaloneImageSource(source);
  if (!normalized) return false;
  return !DIRECT_IMAGE_SOURCE_RE.test(normalized);
}

export function markdownImageDirectSrc(source: string): string {
  const normalized = trimStandaloneImageSource(source);
  if (normalized.startsWith("//")) return `https:${normalized}`;
  return normalized;
}

function renderMarkdownImage(source: string, alt = ""): string {
  const normalized = trimStandaloneImageSource(source);
  const escapedSource = escapeAttr(normalized);
  const escapedAlt = escapeAttr(alt);
  const resolved = !shouldResolveMarkdownImageSource(normalized);
  const state = resolved ? "ready" : "pending";
  const srcAttr = resolved ? ` src="${escapeAttr(markdownImageDirectSrc(normalized))}"` : "";
  return `<span class="md-image-frame" data-md-image-frame="true" data-md-image-source="${escapedSource}" data-md-image-state="${state}" title="${escapedSource}"><img class="md-image-preview" data-md-image-source="${escapedSource}" data-md-image-state="${state}"${srcAttr} alt="${escapedAlt}" loading="lazy" draggable="false"></span>`;
}

function replaceMarkdownImageTags(html: string): string {
  return html.replace(/<img\b[^>]*>/gi, (tag) => {
    const source = attrValue(tag, "src");
    if (!source) return tag;
    return renderMarkdownImage(source, attrValue(tag, "alt"));
  });
}

function replaceAutolinkedImageUrls(html: string): string {
  return html.replace(/<a\b([^>]*)\bhref\s*=\s*("([^"]*)"|'([^']*)'|([^\s"'>]+))([^>]*)>([\s\S]*?)<\/a>/gi, (match, before, _hrefAttr, hrefDouble, hrefSingle, hrefBare, after, label) => {
    const href = decodeHtml(hrefDouble ?? hrefSingle ?? hrefBare ?? "");
    const decodedLabel = decodeHtml(label.replace(/<[^>]+>/g, "").trim());
    if (!isMarkdownImageSource(href) || decodedLabel !== href) return match;
    if (before.includes(">") || after.includes(">")) return match;
    return renderMarkdownImage(href);
  });
}

function replaceStandaloneImagePaths(html: string): string {
  return walkHtmlText(html, (text) => {
    const match = text.match(/^(\s*)(.+?)(\s*)$/s);
    if (!match) return text;
    const source = trimStandaloneImageSource(match[2]);
    if (!isMarkdownImageSource(source)) return text;
    return `${match[1]}${renderMarkdownImage(source)}${match[3]}`;
  });
}

export function prepareMarkdownImages(html: string): string {
  if (!html) return "";
  return replaceStandaloneImagePaths(
    replaceAutolinkedImageUrls(
      replaceMarkdownImageTags(html),
    ),
  );
}
