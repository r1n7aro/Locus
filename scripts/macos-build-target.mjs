// Keep the requested target independent of the machine running this script.
export function resolveMacosTarget(args = process.argv.slice(2), env = process.env, host = process.platform, arch = process.arch) {
  const index = args.findIndex((arg) => arg === "--target" || arg === "-t");
  const target = (index >= 0 ? args[index + 1] : undefined)
    ?? args.find((arg) => arg.startsWith("--target="))?.slice(9)
    ?? args.find((arg) => arg.startsWith("-t="))?.slice(3)
    ?? env.TAURI_ENV_TARGET_TRIPLE ?? env.LOCUS_MACOS_TARGET ?? env.CARGO_BUILD_TARGET;
  if (target) {
    if (target === "aarch64-apple-darwin") return { triple: target, arch: "arm64" };
    if (target === "x86_64-apple-darwin") return { triple: target, arch: "x64" };
    if (target.includes("apple-darwin")) throw new Error(`Unsupported macOS target: ${target}; build each supported target separately.`);
    return null;
  }
  if (host !== "darwin") return null;
  if (arch === "arm64") return { triple: "aarch64-apple-darwin", arch: "arm64" };
  if (arch === "x64") return { triple: "x86_64-apple-darwin", arch: "x64" };
  throw new Error(`Unsupported macOS host architecture: ${arch}`);
}
