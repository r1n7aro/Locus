export function resolveMacosTarget(args?: string[], env?: Record<string, string | undefined>, host?: string, arch?: string): { triple: string; arch: "arm64" | "x64" } | null;
