export interface PluginRegistrySource {
  id: string;
  name: string;
  owner: string;
  repo: string;
  url?: string;
  branch: string;
  path: string;
}

export interface ParsedPluginRegistryRepo {
  owner: string;
  repo: string;
  url: string;
  provider: "github" | "gitlab" | "gitee" | "gitea" | "raw";
  branch?: string;
  path?: string;
}

export const DEFAULT_PLUGIN_REGISTRY_BRANCH = "main";
export const DEFAULT_PLUGIN_REGISTRY_PATH = "public/v1";
export const DEFAULT_PLUGIN_REGISTRY_SOURCE: PluginRegistrySource = {
  id: "default",
  name: "Locus Registry",
  owner: "r1n7aro",
  repo: "locus-plugin-registry",
  branch: DEFAULT_PLUGIN_REGISTRY_BRANCH,
  path: DEFAULT_PLUGIN_REGISTRY_PATH,
};

const GITHUB_OWNER_REPO_PATTERN = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const LEGACY_OFFICIAL_PLUGIN_REGISTRY_PATH = "v1";

function trimGitSuffix(value: string): string {
  return value.trim().replace(/\.git$/i, "");
}

function trimSlashes(value: string): string {
  return value.trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
}

function splitBranchPath(segments: string[]): { branch?: string; path?: string } {
  const [branch, ...pathSegments] = segments.map((segment) => decodeURIComponent(segment)).filter(Boolean);
  return {
    branch,
    path: pathSegments.length ? pathSegments.join("/") : undefined,
  };
}

function repoPathLabel(url: URL, segments: string[]): string {
  return `${url.origin}/${segments.map((segment) => encodeURIComponent(decodeURIComponent(segment))).join("/")}`;
}

function parsedFromRepoSegments(
  url: URL,
  segments: string[],
  provider: ParsedPluginRegistryRepo["provider"],
): ParsedPluginRegistryRepo | null {
  const cleanSegments = segments.map((segment) => decodeURIComponent(segment)).filter(Boolean);
  if (cleanSegments.length < 2) return null;
  cleanSegments[cleanSegments.length - 1] = trimGitSuffix(cleanSegments[cleanSegments.length - 1]);
  return {
    owner: cleanSegments[0],
    repo: cleanSegments.slice(1).join("/"),
    url: repoPathLabel(url, cleanSegments),
    provider,
  };
}

export function parsePluginRegistryRepoInput(value: string): ParsedPluginRegistryRepo | null {
  const raw = trimGitSuffix(value);
  if (!raw) return null;
  if (GITHUB_OWNER_REPO_PATTERN.test(raw)) {
    const [owner, repo] = raw.split("/");
    return { owner, repo, url: `${owner}/${repo}`, provider: "github" };
  }
  try {
    const url = new URL(raw);
    if (!matchesHttpUrl(url)) return null;
    const host = url.hostname.toLowerCase();
    const segments = url.pathname
      .split("/")
      .map((segment) => segment.trim())
      .filter(Boolean);

    if (host === "raw.githubusercontent.com" && segments.length >= 4) {
      const [owner, repo, branch, ...pathSegments] = segments.map((segment) => decodeURIComponent(segment));
      return {
        owner,
        repo,
        url: raw.trimEnd().replace(/\/+$/g, ""),
        provider: "raw",
        branch,
        path: pathSegments.length ? pathSegments.join("/") : undefined,
      };
    }

    if (host === "github.com" && segments.length >= 2) {
      const [owner, repoWithSuffix, mode, ...rest] = segments;
      const repo = trimGitSuffix(decodeURIComponent(repoWithSuffix));
      if (!owner || !repo) return null;
      const parsed = splitBranchPath(mode === "tree" || mode === "raw" || mode === "blob" ? rest : []);
      return {
        owner: decodeURIComponent(owner),
        repo,
        url: `${url.origin}/${encodeURIComponent(decodeURIComponent(owner))}/${encodeURIComponent(repo)}`,
        provider: "github",
        ...parsed,
      };
    }

    const markerIndex = segments.findIndex((segment) => segment === "-");
    if (markerIndex > 1 && (segments[markerIndex + 1] === "tree" || segments[markerIndex + 1] === "raw" || segments[markerIndex + 1] === "blob")) {
      const repoSegments = segments.slice(0, markerIndex);
      const parsed = parsedFromRepoSegments(url, repoSegments, "gitlab");
      if (!parsed) return null;
      return {
        ...parsed,
        ...splitBranchPath(segments.slice(markerIndex + 2)),
      };
    }

    if (host === "gitee.com" && segments.length >= 2) {
      const [owner, repoWithSuffix, mode, ...rest] = segments;
      const repo = trimGitSuffix(decodeURIComponent(repoWithSuffix));
      const parsed = splitBranchPath(mode === "tree" || mode === "raw" || mode === "blob" ? rest : []);
      return {
        owner: decodeURIComponent(owner),
        repo,
        url: `${url.origin}/${encodeURIComponent(decodeURIComponent(owner))}/${encodeURIComponent(repo)}`,
        provider: "gitee",
        ...parsed,
      };
    }

    if (host.includes("gitlab") && segments.length >= 2) {
      return parsedFromRepoSegments(url, segments, "gitlab");
    }

    const rawIndex = segments.findIndex((segment) => segment === "raw");
    const branchIndex = rawIndex >= 0 && segments[rawIndex + 1] === "branch" ? rawIndex + 1 : -1;
    if (rawIndex > 1 || branchIndex > 1) {
      const repoSegments = segments.slice(0, rawIndex);
      const parsed = parsedFromRepoSegments(url, repoSegments, "gitea");
      if (!parsed) return null;
      return {
        ...parsed,
        ...splitBranchPath(segments.slice(rawIndex + (branchIndex > 0 ? 2 : 1))),
      };
    }

    const srcIndex = segments.findIndex((segment) => segment === "src");
    const srcBranchIndex = srcIndex >= 0 && segments[srcIndex + 1] === "branch" ? srcIndex + 1 : -1;
    if (srcBranchIndex > 1) {
      const repoSegments = segments.slice(0, srcIndex);
      const parsed = parsedFromRepoSegments(url, repoSegments, "gitea");
      if (!parsed) return null;
      return {
        ...parsed,
        ...splitBranchPath(segments.slice(srcBranchIndex + 1)),
      };
    }

    return {
      owner: url.hostname,
      repo: trimSlashes(url.pathname) || url.hostname,
      url: raw.trimEnd().replace(/\/+$/g, ""),
      provider: "raw",
    };
  } catch {
    return null;
  }
}

function matchesHttpUrl(url: URL): boolean {
  return url.protocol === "https:" || url.protocol === "http:";
}

export function normalizePluginRegistryBranch(value: string): string {
  return value.trim() || DEFAULT_PLUGIN_REGISTRY_BRANCH;
}

export function normalizePluginRegistryPath(value: string): string | null {
  const normalized = trimSlashes(value);
  if (!normalized) return null;
  const segments = normalized.split("/");
  if (segments.some((segment) => !segment || segment === "." || segment === ".." || segment.includes(":"))) {
    return null;
  }
  return normalized;
}

function isOfficialPluginRegistryRepo(owner: string, repo: string): boolean {
  return owner.toLowerCase() === DEFAULT_PLUGIN_REGISTRY_SOURCE.owner.toLowerCase()
    && repo.toLowerCase() === DEFAULT_PLUGIN_REGISTRY_SOURCE.repo.toLowerCase();
}

function normalizePluginRegistrySourcePath(
  parsed: ParsedPluginRegistryRepo,
  rawPath: string,
): string | null {
  const path = normalizePluginRegistryPath(rawPath);
  if (!path) return null;
  if (isOfficialPluginRegistryRepo(parsed.owner, parsed.repo) && path === LEGACY_OFFICIAL_PLUGIN_REGISTRY_PATH) {
    return DEFAULT_PLUGIN_REGISTRY_PATH;
  }
  return path;
}

export function pluginRegistrySourceRepoLabel(source: PluginRegistrySource): string {
  if (source.url?.trim()) return source.url.trim();
  return `${source.owner}/${source.repo}`;
}

export function pluginRegistrySourceBaseUrl(source: PluginRegistrySource): string {
  const parsed = parsePluginRegistryRepoInput(pluginRegistrySourceRepoLabel(source));
  if (!parsed) return DEFAULT_PLUGIN_REGISTRY_SOURCE.url
    ? pluginRegistrySourceBaseUrl(DEFAULT_PLUGIN_REGISTRY_SOURCE)
    : `https://raw.githubusercontent.com/${DEFAULT_PLUGIN_REGISTRY_SOURCE.owner}/${DEFAULT_PLUGIN_REGISTRY_SOURCE.repo}/${DEFAULT_PLUGIN_REGISTRY_BRANCH}/${DEFAULT_PLUGIN_REGISTRY_PATH}`;
  if (parsed.provider === "raw") {
    return parsed.url.trimEnd().replace(/\/+$/g, "");
  }
  const branch = encodeURIComponent(parsed.branch ?? normalizePluginRegistryBranch(source.branch));
  const path = normalizePluginRegistrySourcePath(parsed, source.path) ?? DEFAULT_PLUGIN_REGISTRY_PATH;
  const encodedPath = path.split("/").map((segment) => encodeURIComponent(segment)).join("/");
  if (parsed.provider === "gitlab") {
    return `${parsed.url}/-/raw/${branch}/${encodedPath}`;
  }
  if (parsed.provider === "gitee") {
    return `${parsed.url}/raw/${branch}/${encodedPath}`;
  }
  if (parsed.provider === "gitea") {
    return `${parsed.url}/raw/branch/${branch}/${encodedPath}`;
  }
  const owner = encodeURIComponent(parsed.owner);
  const repo = parsed.repo.split("/").map((segment) => encodeURIComponent(segment)).join("/");
  return `https://raw.githubusercontent.com/${owner}/${repo}/${branch}/${encodedPath}`;
}

export function normalizePluginRegistrySource(value: Partial<PluginRegistrySource>): PluginRegistrySource | null {
  const input = value.url?.trim() || (
    value.owner?.trim() && value.repo?.trim()
      ? `${value.owner.trim()}/${value.repo.trim()}`
      : ""
  );
  const parsed = parsePluginRegistryRepoInput(input);
  if (!parsed) return null;
  const branch = normalizePluginRegistryBranch(value.branch ?? parsed.branch ?? "");
  const path = normalizePluginRegistrySourcePath(parsed, value.path ?? parsed.path ?? DEFAULT_PLUGIN_REGISTRY_PATH);
  if (!path) return null;
  const url = parsed.provider === "raw" && isOfficialPluginRegistryRepo(parsed.owner, parsed.repo)
    ? `${parsed.owner}/${parsed.repo}`
    : parsed.url;
  const id = value.id?.trim() || `registry-${Date.now().toString(36)}`;
  const name = value.name?.trim() || pluginRegistrySourceRepoLabel({
    id,
    name: "",
    owner: parsed.owner,
    repo: parsed.repo,
    url,
    branch,
    path,
  });
  return { id, name, owner: parsed.owner, repo: parsed.repo, url, branch, path };
}

export function normalizePluginRegistrySources(values: unknown): PluginRegistrySource[] {
  if (!Array.isArray(values)) return [DEFAULT_PLUGIN_REGISTRY_SOURCE];
  const sources = values
    .map((value) => normalizePluginRegistrySource(value as Partial<PluginRegistrySource>))
    .filter((value): value is PluginRegistrySource => !!value);
  if (!sources.length) return [DEFAULT_PLUGIN_REGISTRY_SOURCE];
  const seen = new Set<string>();
  return sources.filter((source) => {
    if (seen.has(source.id)) return false;
    seen.add(source.id);
    return true;
  });
}
