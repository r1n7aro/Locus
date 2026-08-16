type ToolCallArguments = Record<string, unknown>;

const MCP_TOOL_PREFIX = "mcp__";

export interface ToolPathSummaryContext {
  workingDir?: string;
  extraWorkdirs?: readonly string[];
}

interface NormalizedPath {
  value: string;
  prefix: string;
  segments: string[];
  absolute: boolean;
  caseInsensitive: boolean;
}

export interface McpToolNameParts {
  serverId: string;
  toolName: string;
}

/// `mcp__<server>__<tool>` → parts; null for non-MCP names. Best-effort
/// string split — the backend owns the authoritative wire-name map.
export function parseMcpToolName(name: string): McpToolNameParts | null {
  if (!name.startsWith(MCP_TOOL_PREFIX)) return null;
  const rest = name.slice(MCP_TOOL_PREFIX.length);
  const sep = rest.indexOf("__");
  if (sep <= 0 || sep + 2 >= rest.length) {
    return { serverId: "", toolName: rest };
  }
  return { serverId: rest.slice(0, sep), toolName: rest.slice(sep + 2) };
}

/// UI display name for a tool call: MCP wire names shed their
/// `mcp__<server>__` prefix, everything else passes through.
export function toolCallDisplayName(name: string): string {
  return parseMcpToolName(name)?.toolName ?? name;
}

function getStringArg(args: ToolCallArguments, keys: string[]): string {
  for (const key of keys) {
    const value = args[key];
    if (typeof value === "string" && value.length > 0) {
      return value;
    }
  }
  return "";
}

function normalizePath(path: string): NormalizedPath {
  const unified = path.trim().replace(/\\/g, "/");
  let prefix = "";
  let rest = unified;
  let absolute = false;
  let caseInsensitive = false;

  const driveMatch = rest.match(/^([A-Za-z]:)\/+/);
  if (driveMatch) {
    prefix = `${driveMatch[1]}/`;
    rest = rest.slice(driveMatch[0].length);
    absolute = true;
    caseInsensitive = true;
  } else if (rest.startsWith("//")) {
    const uncSegments = rest.slice(2).split("/").filter(Boolean);
    if (uncSegments.length >= 2) {
      prefix = `//${uncSegments[0]}/${uncSegments[1]}/`;
      rest = uncSegments.slice(2).join("/");
    } else {
      prefix = "//";
      rest = uncSegments.join("/");
    }
    absolute = true;
    caseInsensitive = true;
  } else if (rest.startsWith("/")) {
    prefix = "/";
    rest = rest.replace(/^\/+/, "");
    absolute = true;
  }

  const segments: string[] = [];
  for (const segment of rest.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      const previous = segments[segments.length - 1];
      if (previous && previous !== "..") {
        segments.pop();
      } else if (!absolute) {
        segments.push(segment);
      }
      continue;
    }
    segments.push(segment);
  }

  return {
    value: `${prefix}${segments.join("/")}`,
    prefix,
    segments,
    absolute,
    caseInsensitive,
  };
}

function comparablePath(path: NormalizedPath): string {
  const value = path.value === "/" ? "/" : path.value.replace(/\/+$/, "");
  return path.caseInsensitive ? value.toLowerCase() : value;
}

function relativePathUnderRoot(path: NormalizedPath, root: NormalizedPath): string | null {
  if (!path.absolute || !root.absolute) return null;
  const pathKey = comparablePath(path);
  const rootKey = comparablePath(root);
  if (pathKey === rootKey) return "";
  const rootPrefix = rootKey === "/" ? "/" : `${rootKey}/`;
  if (!pathKey.startsWith(rootPrefix)) return null;

  const displayRoot = root.value === "/" ? "/" : root.value.replace(/\/+$/, "");
  return rootKey === "/"
    ? path.value.slice(1)
    : path.value.slice(displayRoot.length + 1);
}

function rootLabel(root: NormalizedPath, roots: readonly NormalizedPath[]): string {
  if (root.segments.length === 0) return root.value;

  for (let depth = 1; depth <= root.segments.length; depth += 1) {
    const candidate = root.segments.slice(-depth).join("/");
    const candidateKey = root.caseInsensitive ? candidate.toLowerCase() : candidate;
    const duplicate = roots.some((other) => {
      if (other === root || other.segments.length < depth) return false;
      const otherCandidate = other.segments.slice(-depth).join("/");
      const otherKey = other.caseInsensitive ? otherCandidate.toLowerCase() : otherCandidate;
      return otherKey === candidateKey;
    });
    if (!duplicate) return candidate;
  }

  return root.value.replace(/\/+$/, "");
}

function compactAbsolutePath(path: NormalizedPath): string {
  if (path.segments.length <= 5) return path.value;
  const head = path.segments.slice(0, 2).join("/");
  const tail = path.segments.slice(-2).join("/");
  return `${path.prefix}${head}/…/${tail}`;
}

function summarizePath(path: string, context: ToolPathSummaryContext): string {
  const normalized = normalizePath(path);
  if (!normalized.value) return "";
  if (!normalized.absolute) return normalized.value;

  const workingDir = normalizePath(context.workingDir ?? "");
  const workspaceRelative = relativePathUnderRoot(normalized, workingDir);
  if (workspaceRelative !== null) {
    return workspaceRelative || rootLabel(workingDir, [workingDir]);
  }

  const extraRoots = (context.extraWorkdirs ?? [])
    .map(normalizePath)
    .filter((root) => root.absolute && root.value.length > 0);
  const extraMatch = extraRoots
    .map((root) => ({ root, relative: relativePathUnderRoot(normalized, root) }))
    .filter(
      (match): match is { root: NormalizedPath; relative: string } => match.relative !== null,
    )
    .sort((left, right) => right.root.value.length - left.root.value.length)[0];
  if (extraMatch) {
    const label = rootLabel(extraMatch.root, extraRoots);
    return extraMatch.relative ? `${label}/${extraMatch.relative}` : label;
  }

  return compactAbsolutePath(normalized);
}

function joinUnityYamlPath(filePath: string, objectPath: string): string {
  const normalizedFilePath = filePath.replace(/\\/g, "/").replace(/\/+$/, "");
  const normalizedObjectPath = objectPath.replace(/\\/g, "/").replace(/^\/+/, "");
  if (normalizedFilePath && normalizedObjectPath) {
    return `${normalizedFilePath}/${normalizedObjectPath}`;
  }
  return normalizedFilePath || normalizedObjectPath;
}

export function buildToolCallArgsSummary(
  toolName: string,
  argumentsText: string,
  pathContext: ToolPathSummaryContext = {},
): string {
  try {
    const args = JSON.parse(argumentsText);
    if (!args || typeof args !== "object" || Array.isArray(args)) return "";

    if (toolName === "tool_search" && Array.isArray(args.wire_names)) {
      return args.wire_names
        .filter((name: unknown): name is string => typeof name === "string" && name.length > 0)
        .join(", ");
    }

    if (toolName === "read" || toolName === "write" || toolName === "edit" || toolName === "list") {
      const p = getStringArg(args, ["filePath", "file_path", "path"]);
      if (!p) return "";
      return summarizePath(p, pathContext);
    }

    if (toolName === "unity_yaml_read" || toolName === "unity_yaml_search") {
      const filePath = getStringArg(args, ["filePath", "file_path", "path"]);
      const objectPath = getStringArg(args, ["objectPath", "object_path"]);
      const targetPath = joinUnityYamlPath(summarizePath(filePath, pathContext), objectPath);
      if (targetPath) return targetPath;
    }

    if (toolName === "grep") {
      const pat = getStringArg(args, ["pattern"]);
      const path = getStringArg(args, ["filePath", "file_path", "path"]);
      if (pat && path) return `/${pat}/ in ${summarizePath(path, pathContext)}`;
      if (pat) return `/${pat}/`;
      return "";
    }

    if (toolName === "bash") {
      const cmd = getStringArg(args, ["command"]);
      if (cmd.length <= 60) return cmd;
      return cmd.slice(0, 57) + "...";
    }

    if (
      toolName === "code_find_references" ||
      toolName === "code_goto_definition" ||
      toolName === "code_hover"
    ) {
      const symbol = getStringArg(args, ["symbol"]);
      const filePath = getStringArg(args, ["filePath", "file_path"]);
      if (symbol && filePath) return `${symbol} @ ${summarizePath(filePath, pathContext)}`;
      return symbol;
    }

    if (toolName === "code_symbol_search") {
      return getStringArg(args, ["query"]);
    }

    if (toolName === "code_diagnostics") {
      const filePath = getStringArg(args, ["filePath", "file_path"]);
      if (filePath) return summarizePath(filePath, pathContext);
      return "workspace";
    }

    if (toolName === "unity_code_usages") {
      const member = getStringArg(args, ["member"]);
      const filePath = getStringArg(args, ["filePath", "file_path"]);
      if (member && filePath) return `${member} @ ${summarizePath(filePath, pathContext)}`;
      if (filePath) return summarizePath(filePath, pathContext);
      return member;
    }

    if (toolName === "subagent" || toolName === "task") {
      const desc = getStringArg(args, ["description"]);
      if (desc.length <= 60) return desc;
      return desc.slice(0, 57) + "...";
    }

    if (toolName === "web_fetch") {
      return getStringArg(args, ["url"]);
    }

    for (const v of Object.values(args)) {
      if (typeof v === "string" && v.length > 0) {
        return v.length <= 60 ? v : v.slice(0, 57) + "...";
      }
    }
    return "";
  } catch {
    return "";
  }
}
