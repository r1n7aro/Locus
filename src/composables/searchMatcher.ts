export interface SearchField {
  text: string;
  weight?: number;
}

function withCamelBoundaries(value: string): string {
  return value.replace(/([a-z0-9])([A-Z])/g, "$1 $2");
}

export function splitSearchTerms(value: string): string[] {
  return withCamelBoundaries(value)
    .replace(/^[@/]+/, "")
    .toLowerCase()
    .split(/[^a-z0-9]+/g)
    .filter(Boolean);
}

function compactSearch(value: string): string {
  return splitSearchTerms(value).join("");
}

function subsequenceGapScore(query: string, candidate: string): number | null {
  if (!query) return 0;

  let queryIndex = 0;
  let lastMatch = -1;
  let gapPenalty = 0;

  for (let i = 0; i < candidate.length && queryIndex < query.length; i += 1) {
    if (candidate[i] !== query[queryIndex]) continue;
    if (lastMatch >= 0) {
      gapPenalty += i - lastMatch - 1;
    }
    lastMatch = i;
    queryIndex += 1;
  }

  if (queryIndex !== query.length) return null;
  return Math.max(0, 260 - gapPenalty * 8);
}

function scoreSingleField(query: string, field: string): number | null {
  const queryTerms = splitSearchTerms(query);
  const compactQuery = queryTerms.join("");
  if (!compactQuery) return 0;

  const candidateTerms = splitSearchTerms(field);
  if (candidateTerms.length === 0) return null;
  const compactCandidate = candidateTerms.join("");

  if (compactCandidate === compactQuery) {
    return 1000;
  }

  if (compactCandidate.startsWith(compactQuery)) {
    return 920 - Math.min(compactCandidate.length, 40);
  }

  if (candidateTerms.some((term) => term === compactQuery)) {
    return 860;
  }

  if (candidateTerms.some((term) => term.startsWith(compactQuery))) {
    return 820;
  }

  const compactIndex = compactCandidate.indexOf(compactQuery);
  if (compactIndex >= 0) {
    return 760 - compactIndex * 4;
  }

  if (queryTerms.length > 1 && queryTerms.every((term) =>
    candidateTerms.some((candidateTerm) => candidateTerm.includes(term))
  )) {
    return 640;
  }

  return subsequenceGapScore(compactQuery, compactCandidate);
}

export function scoreSearchFields(query: string, fields: SearchField[]): number | null {
  let bestScore: number | null = null;

  for (const field of fields) {
    const score = scoreSingleField(query, field.text);
    if (score == null) continue;
    const weighted = score + (field.weight ?? 0);
    if (bestScore == null || weighted > bestScore) {
      bestScore = weighted;
    }
  }

  return bestScore;
}

export function rankSearchResults<T>(
  items: T[],
  query: string,
  fieldsForItem: (item: T) => SearchField[],
): T[] {
  const compactQuery = compactSearch(query);
  if (!compactQuery) return items;

  return items
    .map((item, index) => ({
      item,
      index,
      score: scoreSearchFields(query, fieldsForItem(item)),
    }))
    .filter((entry) => entry.score != null)
    .sort((a, b) => {
      if (b.score !== a.score) return (b.score ?? 0) - (a.score ?? 0);
      return a.index - b.index;
    })
    .map((entry) => entry.item);
}
