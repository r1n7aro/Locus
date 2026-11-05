import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import path from "node:path";

export function clientEdition(env = process.env) {
  const edition = env.LOCUS_BUILD_EDITION || "public";
  if (edition !== "public" && edition !== "internal") throw new Error("Invalid LOCUS_BUILD_EDITION");
  return edition;
}

export function assertPublicArguments(args, env = process.env, repoRoot = process.cwd()) {
  if (clientEdition(env) !== "public") throw new Error("Public release cannot enable the internal client");
  const text = args.join(" ");
  const injectedFeature = Object.entries(env).some(([name, value]) =>
    /^(?:RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_RUSTFLAGS|CARGO_TARGET_.+_RUSTFLAGS)$/i.test(name)
      && /server[-_]client/i.test(String(value)));
  if (/server-client|--all-features|tauri\.internal|dist-internal/.test(text)
      || env.CARGO_FEATURE_SERVER_CLIENT || injectedFeature) {
    throw new Error("Public release cannot include server-client features or internal assets");
  }
  const checkOverlay = (value) => {
    let overlay;
    try {
      overlay = JSON.parse(value.trim().startsWith("{") ? value : readFileSync(path.resolve(repoRoot, value), "utf8"));
    } catch {
      throw new Error("Public builds require a readable JSON configuration overlay");
    }
    const output = overlay.build?.frontendDist;
    if ((output !== undefined && output !== "../dist")
        || /server-client|dist-internal|internal:/.test(JSON.stringify(overlay))) {
      throw new Error("Public configuration cannot change the frontend edition or enable internal features");
    }
  };
  for (let index = 0; index < args.length; index++) {
    if (args[index] === "--config" || args[index] === "-c") {
      const value = args[++index];
      if (!value) throw new Error("Missing configuration overlay");
      checkOverlay(value);
    } else if (args[index].startsWith("--config=")) checkOverlay(args[index].slice("--config=".length));
  }
  if (env.TAURI_CONFIG) checkOverlay(env.TAURI_CONFIG);
}

export function assertClientBundle(repoRoot, edition = "public") {
  const root = path.join(repoRoot, edition === "internal" ? "dist-internal" : "dist");
  const marker = path.join(root, ".locus-edition.json");
  if (!existsSync(marker)) throw new Error("Frontend edition manifest is missing; rebuild the frontend");
  const manifest = JSON.parse(readFileSync(marker, "utf8"));
  if (manifest.edition !== edition || !Array.isArray(manifest.files)) throw new Error("Frontend edition mismatch");
  const files = [];
  function visit(dir) {
    for (const name of readdirSync(dir)) {
      const file = path.join(dir, name);
      if (statSync(file).isDirectory()) visit(file);
      else files.push(path.relative(root, file).replaceAll("\\", "/"));
    }
  }
  visit(root);
  const allowed = new Set([...manifest.files, ".locus-edition.json"]);
  let internalImplementation = false;
  for (const file of files) {
    if (!allowed.has(file)) throw new Error(`Untracked frontend artifact: ${file}`);
    if (/\.(?:js|map|html)$/.test(file)
        && /server_client_[a-z_]+\b/.test(readFileSync(path.join(root, file), "utf8"))) {
      internalImplementation = true;
      if (edition === "public") throw new Error(`Internal client implementation in public artifact: ${file}`);
    }
  }
  if (edition === "internal" && !internalImplementation) throw new Error("Internal client implementation is missing from the artifact");
}
