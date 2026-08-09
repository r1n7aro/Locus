const CONTEXT_EXPORT_TITLE_MAX_CHARS = 72;

export function sessionContextExportTitleFragment(title: string): string {
  const normalized = title
    .normalize("NFKC")
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")
    .replace(/\s+/g, "_")
    .replace(/_+/g, "_")
    .replace(/^[._\s]+|[._\s]+$/g, "");
  return Array.from(normalized).slice(0, CONTEXT_EXPORT_TITLE_MAX_CHARS).join("") || "untitled";
}

export function sessionContextExportFileName(sessionId: string, title: string): string {
  const shortId = sessionId.trim().slice(0, 8) || "session";
  return `context_${shortId}_${sessionContextExportTitleFragment(title)}.yaml`;
}
