/**
 * Pure functions for injecting interactive elements (asset chips, file refs)
 * into rendered Markdown HTML. Extracted for testability.
 */

/**
 * Walk HTML string, applying `transform` only to text segments outside
 * code/pre blocks and anchor tags. Tags and protected content pass through.
 */
export function walkHtmlText(html: string, transform: (text: string) => string): string {
  const parts = html.split(/(<[^>]+>)/);
  let inCode = 0;
  let inAnchor = 0;
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    if (part.startsWith("<")) {
      if (/^<(code|pre)[\s>]/i.test(part)) inCode++;
      else if (/^<\/(code|pre)>/i.test(part)) inCode = Math.max(0, inCode - 1);
      if (/^<a[\s>]/i.test(part)) inAnchor++;
      else if (/^<\/a>/i.test(part)) inAnchor = Math.max(0, inAnchor - 1);
      continue;
    }
    if (inCode > 0 || inAnchor > 0) continue;
    parts[i] = transform(part);
  }
  return parts.join("");
}

const ASSET_REF_RE = /@((?:Assets|Packages|ProjectSettings)\/(?:[^\s@<]+\/)*[^\s@<]+)/g;
const WORKSPACE_MENTION_RE = /@((?:[^\s@<]+\/)+[^\s@<]*)/g;

export function injectAssetChips(html: string): string {
  return walkHtmlText(html, (text) =>
    text.replace(ASSET_REF_RE, (_match, path) => {
      const escaped = path.replace(/"/g, "&quot;");
      const segments = path.split("/");
      const fileName = segments[segments.length - 1] || path;
      const dotIdx = fileName.lastIndexOf(".");
      const name = dotIdx > 0 ? fileName.substring(0, dotIdx) : fileName;
      return `<span class="md-asset-chip ui-select-text" data-asset-path="${escaped}" title="${escaped}"><span class="md-asset-chip-icon">\u25C7</span>${name}</span>`;
    }),
  );
}

export function injectWorkspaceMentions(html: string): string {
  return walkHtmlText(html, (text) =>
    text.replace(WORKSPACE_MENTION_RE, (match, path) => {
      const isDir = path.endsWith("/");
      if (/^(Assets|Packages|ProjectSettings)\//.test(path) && !isDir) {
        return match;
      }

      const normalizedPath = path.replace(/\/+$/, "");
      if (!normalizedPath) {
        return match;
      }

      const escapedPath = normalizedPath.replace(/"/g, "&quot;");
      const segments = normalizedPath.split("/").filter(Boolean);
      const name = segments[segments.length - 1] || normalizedPath;
      const title = `${escapedPath}${isDir ? "/" : ""}`;
      const fileAttr = isDir ? "" : ` data-file-path="${escapedPath}"`;
      const classes = isDir ? "md-workspace-ref md-folder-ref" : "md-workspace-ref md-file-ref";

      return `<span class="${classes} ui-select-text" data-workspace-path="${escapedPath}" data-entry-kind="${isDir ? "folder" : "file"}"${fileAttr} title="${title}"><span class="md-workspace-ref-prefix">@</span>${name}${isDir ? "/" : ""}</span>`;
    }),
  );
}

// Match project-relative file paths, optionally with :line or #Lline suffix.
// Requires at least one slash and a file extension to reduce false positives.
// Does not match if preceded by @ (already handled as a workspace mention) or quotes/backticks.
const FILE_REF_RE = /(?<![@"'`\/])(?:(?:src|src-tauri|Assets|Packages|Library|ProjectSettings|Editor)\/[\w.\/\-]+[\w.\-]|[\w.\-]+\/[\w.\/\-]*\.[\w]+)(?::(\d+)|#L(\d+))?/g;

// Detects if a match is inside a URL by checking preceding text for ://
const URL_CONTEXT_RE = /\w+:\/\/\S*$/;

export function injectFileRefs(html: string): string {
  return walkHtmlText(html, (text) => {
    // Skip text inside already-injected asset chips (data-asset-path attr content)
    if (text.includes("data-asset-path") || text.includes("data-workspace-path")) return text;
    return text.replace(FILE_REF_RE, (match, lineColon, lineHash, offset, fullText) => {
      // Skip matches that are part of a URL
      const preceding = fullText.slice(0, offset);
      if (URL_CONTEXT_RE.test(preceding)) return match;
      const line = lineColon || lineHash || "";
      // Strip line suffix to get clean file path
      let filePath = match;
      if (lineColon) filePath = match.slice(0, match.lastIndexOf(":" + lineColon));
      else if (lineHash) filePath = match.slice(0, match.lastIndexOf("#L" + lineHash));
      const escaped = filePath.replace(/"/g, "&quot;");
      const segments = filePath.split("/");
      const fileName = segments[segments.length - 1] || filePath;
      const displayText = line ? `${fileName}:${line}` : fileName;
      const lineAttr = line ? ` data-file-line="${line}"` : "";
      return `<span class="md-file-ref ui-select-text" data-file-path="${escaped}"${lineAttr} title="${escaped}${line ? ":" + line : ""}">\u25A1 ${displayText}</span>`;
    });
  });
}
