type ToolCallArguments = Record<string, unknown>;

function getStringArg(args: ToolCallArguments, keys: string[]): string {
  for (const key of keys) {
    const value = args[key];
    if (typeof value === "string" && value.length > 0) {
      return value;
    }
  }
  return "";
}

function shortenPath(p: string): string {
  const parts = p.replace(/\\/g, "/").split("/").filter(Boolean);
  if (parts.length <= 2) return parts.join("/");
  return "…/" + parts.slice(-2).join("/");
}

function joinUnityYamlPath(filePath: string, objectPath: string): string {
  const normalizedFilePath = filePath.replace(/\\/g, "/").replace(/\/+$/, "");
  const normalizedObjectPath = objectPath.replace(/\\/g, "/").replace(/^\/+/, "");
  if (normalizedFilePath && normalizedObjectPath) {
    return `${normalizedFilePath}/${normalizedObjectPath}`;
  }
  return normalizedFilePath || normalizedObjectPath;
}

export function buildToolCallArgsSummary(toolName: string, argumentsText: string): string {
  try {
    const args = JSON.parse(argumentsText);
    if (!args || typeof args !== "object" || Array.isArray(args)) return "";

    if (toolName === "read" || toolName === "write" || toolName === "edit" || toolName === "list") {
      const p = getStringArg(args, ["filePath", "file_path", "path"]);
      if (!p) return "";
      return shortenPath(p);
    }

    if (toolName === "unity_yaml_read") {
      const filePath = getStringArg(args, ["filePath", "file_path", "path"]);
      const objectPath = getStringArg(args, ["objectPath", "object_path"]);
      const targetPath = joinUnityYamlPath(filePath, objectPath);
      if (targetPath) return targetPath;
    }

    if (toolName === "grep") {
      const pat = getStringArg(args, ["pattern"]);
      const path = getStringArg(args, ["filePath", "file_path", "path"]);
      if (pat && path) return `/${pat}/ in ${shortenPath(path)}`;
      if (pat) return `/${pat}/`;
      return "";
    }

    if (toolName === "bash") {
      const cmd = getStringArg(args, ["command"]);
      if (cmd.length <= 60) return cmd;
      return cmd.slice(0, 57) + "...";
    }

    if (toolName === "code_find_references" || toolName === "code_goto_definition") {
      const symbol = getStringArg(args, ["symbol"]);
      const filePath = getStringArg(args, ["filePath", "file_path"]);
      if (symbol && filePath) return `${symbol} @ ${shortenPath(filePath)}`;
      return symbol;
    }

    if (toolName === "code_symbol_search") {
      return getStringArg(args, ["query"]);
    }

    if (toolName === "task") {
      const desc = getStringArg(args, ["description"]);
      if (desc.length <= 60) return desc;
      return desc.slice(0, 57) + "...";
    }

    if (toolName === "graph_view") {
      const title = getStringArg(args, ["title"]);
      if (title.length <= 60) return title;
      return title.slice(0, 57) + "...";
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
