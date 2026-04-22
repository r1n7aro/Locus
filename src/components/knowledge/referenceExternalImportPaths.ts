export interface StableExternalImportTargetPathInput {
  fixedTargetPath?: string | null;
  preferredTargetPath?: string | null;
  materializedTargetPath?: string | null;
  basePath?: string | null;
  pathExists?: ((path: string) => boolean) | null;
}

export function buildUniquePath(
  basePath: string,
  pathExists?: ((path: string) => boolean) | null,
): string {
  const normalizedBase = basePath.trim();
  if (!normalizedBase || !pathExists) return normalizedBase;
  if (!pathExists(normalizedBase)) return normalizedBase;
  const segments = normalizedBase.split("/").filter(Boolean);
  const name = segments.pop() || normalizedBase;
  const parentDir = segments.join("/");
  for (let index = 2; index < 1000; index += 1) {
    const nextName = `${name}-${index}`;
    const candidate = parentDir ? `${parentDir}/${nextName}` : nextName;
    if (!pathExists(candidate)) return candidate;
  }
  return normalizedBase;
}

export function resolveStableExternalImportTargetPath(
  input: StableExternalImportTargetPathInput,
): string {
  const fixedTargetPath = input.fixedTargetPath?.trim() || "";
  if (fixedTargetPath) return fixedTargetPath;

  const preferredTargetPath = input.preferredTargetPath?.trim() || "";
  if (preferredTargetPath) return preferredTargetPath;

  const materializedTargetPath = input.materializedTargetPath?.trim() || "";
  if (materializedTargetPath) return materializedTargetPath;

  return buildUniquePath(input.basePath?.trim() || "", input.pathExists);
}
