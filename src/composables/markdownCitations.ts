import type { Citation } from "../types";
import { normalizeExternalMarkdownHref } from "./markdownExternalLinks";

const PRIVATE_CITATION_MARKER_RE = /\uE200cite(?:\uE202[^\uE201]*)\uE201/gu;
const LOSSY_CITATION_MARKER_RE = /\uFFFDcite\uFFFD(?:turn[\w-]+(?:\uFFFDturn[\w-]+)*)\uFFFD/gu;

interface CitationMarker {
  start: number;
  end: number;
  referenceIds: string[];
}

interface NumberedCitation {
  citation: Citation;
  number: number;
}

interface SourceReplacement {
  start: number;
  end: number;
  html: string;
}

function escapeHtmlAttribute(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function referenceIdsFromMarker(marker: string): string[] {
  return marker
    .replace(/^\uE200cite\uE202/u, "")
    .replace(/^\uFFFDcite\uFFFD/u, "")
    .replace(/[\uE201\uFFFD]$/u, "")
    .split(/[\uE202\uFFFD]/u)
    .map((value) => value.trim())
    .filter((value) => value.startsWith("turn"));
}

function collectCitationMarkers(source: string): CitationMarker[] {
  const markers: CitationMarker[] = [];
  for (const regex of [PRIVATE_CITATION_MARKER_RE, LOSSY_CITATION_MARKER_RE]) {
    regex.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = regex.exec(source)) !== null) {
      markers.push({
        start: match.index,
        end: match.index + match[0].length,
        referenceIds: referenceIdsFromMarker(match[0]),
      });
    }
  }
  return markers.sort((left, right) => left.start - right.start);
}

function citationPosition(citation: Citation, fallback: number): number {
  const position = citation.endIndex ?? citation.startIndex;
  if (typeof position !== "number" || !Number.isFinite(position)) return fallback;
  return Math.max(0, Math.min(fallback, Math.trunc(position)));
}

function citationOverlapsMarker(citation: Citation, marker: CitationMarker): boolean {
  const start = citation.startIndex;
  const end = citation.endIndex ?? start;
  if (
    typeof start !== "number"
    || typeof end !== "number"
    || !Number.isFinite(start)
    || !Number.isFinite(end)
  ) return false;
  if (start === end) return start >= marker.start && start <= marker.end;
  return start < marker.end && end > marker.start;
}

function citationMatchesMarker(citation: Citation, marker: CitationMarker): boolean {
  const citationReferences = citation.referenceIds ?? [];
  if (
    citationReferences.length > 0
    && marker.referenceIds.some((referenceId) => citationReferences.includes(referenceId))
  ) {
    return true;
  }
  return citationOverlapsMarker(citation, marker);
}

function normalizedCitationUrl(citation: Citation): string | null {
  const normalized = normalizeExternalMarkdownHref(citation.url);
  if (!normalized) return null;
  try {
    const protocol = new URL(normalized).protocol;
    return protocol === "http:" || protocol === "https:" ? normalized : null;
  } catch {
    return null;
  }
}

function citationLabel(citation: Citation, url: string | null): string {
  const explicit = citation.title?.trim() || citation.filename?.trim();
  if (explicit) return explicit;
  if (url) {
    try {
      return new URL(url).hostname;
    } catch {
      return url;
    }
  }
  return citation.referenceIds?.join(", ") || citation.id;
}

function citationHtml(entry: NumberedCitation): string {
  const { citation, number } = entry;
  const url = normalizedCitationUrl(citation);
  const label = citationLabel(citation, url);
  const common = `class="md-citation" data-md-citation-id="${escapeHtmlAttribute(citation.id)}" title="${escapeHtmlAttribute(label)}" aria-label="${escapeHtmlAttribute(`${number}: ${label}`)}"`;
  const content = `<sup>[${number}]</sup>`;
  return url
    ? `<a ${common} href="${escapeHtmlAttribute(url)}">${content}</a>`
    : `<span ${common}>${content}</span>`;
}

function applyReplacements(source: string, replacements: SourceReplacement[]): string {
  const ordered = [...replacements].sort((left, right) => {
    return right.start - left.start || (right.end - right.start) - (left.end - left.start);
  });
  let output = source;
  for (const replacement of ordered) {
    output = output.slice(0, replacement.start) + replacement.html + output.slice(replacement.end);
  }
  return output;
}

/**
 * Converts structured Responses citations into sanitized inline HTML before
 * Markdown parsing. Provider-private cite markers are always consumed, so a
 * streaming or historical response never exposes protocol delimiters.
 */
export function prepareMarkdownCitations(
  source: string,
  citations: readonly Citation[] = [],
): string {
  if (!source) return "";
  const markers = collectCitationMarkers(source);
  const numbered = citations.map((citation, index): NumberedCitation => ({
    citation,
    number: index + 1,
  }));
  const assigned = new Set<string>();
  const replacements: SourceReplacement[] = [];

  for (const marker of markers) {
    let matches = numbered.filter((entry) => (
      !assigned.has(entry.citation.id) && citationMatchesMarker(entry.citation, marker)
    ));
    if (matches.length === 0) {
      const next = numbered.find((entry) => !assigned.has(entry.citation.id));
      if (next) {
        const position = citationPosition(next.citation, source.length);
        if (Math.abs(position - marker.start) <= marker.end - marker.start + 2) {
          matches = [next];
        }
      }
    }
    for (const match of matches) assigned.add(match.citation.id);
    replacements.push({
      start: marker.start,
      end: marker.end,
      html: matches.map(citationHtml).join(""),
    });
  }

  const insertionGroups = new Map<number, NumberedCitation[]>();
  for (const entry of numbered) {
    if (assigned.has(entry.citation.id)) continue;
    const position = citationPosition(entry.citation, source.length);
    const group = insertionGroups.get(position) ?? [];
    group.push(entry);
    insertionGroups.set(position, group);
  }
  for (const [position, entries] of insertionGroups) {
    replacements.push({
      start: position,
      end: position,
      html: entries.map(citationHtml).join(""),
    });
  }

  return applyReplacements(source, replacements);
}
