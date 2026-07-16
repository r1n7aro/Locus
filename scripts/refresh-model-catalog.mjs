#!/usr/bin/env node
// Refreshes the bundled models.dev catalog snapshot used by the model picker.
//
//   bun run catalog:refresh            # fetch https://models.dev/api.json
//   bun run catalog:refresh -- --url https://mirror.example.com/api.json
//
// Output: src-tauri/assets/model_catalog.json.gz (checked in; embedded into the
// binary via include_bytes!). Keep the slim schema in sync with
// src-tauri/src/model_catalog.rs and src/types.ts (ModelCatalog*).

import { gzipSync, gunzipSync } from "node:zlib";
import { mkdirSync, writeFileSync, readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outPath = join(root, "src-tauri", "assets", "model_catalog.json.gz");

const urlArgIndex = process.argv.indexOf("--url");
const sourceUrl =
  urlArgIndex >= 0 && process.argv[urlArgIndex + 1]
    ? process.argv[urlArgIndex + 1]
    : "https://models.dev/api.json";

function slimModel(m) {
  const out = {
    name: m.name ?? m.id,
    limit: {
      context: m.limit?.context ?? 0,
      output: m.limit?.output ?? 0,
    },
  };
  if (m.reasoning) out.reasoning = true;
  if (m.tool_call) out.tool_call = true;
  if (m.attachment) out.attachment = true;
  if (m.temperature) out.temperature = true;
  if (m.interleaved !== undefined) out.interleaved = m.interleaved;
  if (Array.isArray(m.reasoning_options) && m.reasoning_options.length > 0) {
    out.reasoning_options = m.reasoning_options;
  }
  if (m.modalities?.input?.length) out.modalities = { input: m.modalities.input };
  if (m.release_date) out.release_date = m.release_date;
  if (m.status) out.status = m.status;
  if (m.cost && (m.cost.input || m.cost.output)) {
    out.cost = { input: m.cost.input ?? 0, output: m.cost.output ?? 0 };
    if (m.cost.cache_read !== undefined) out.cost.cache_read = m.cost.cache_read;
    if (m.cost.cache_write !== undefined) out.cost.cache_write = m.cost.cache_write;
  }
  return out;
}

function slimProvider(p) {
  const out = { name: p.name ?? p.id, models: {} };
  if (p.api) out.api = p.api;
  if (p.npm) out.npm = p.npm;
  if (Array.isArray(p.env) && p.env.length > 0) out.env = p.env;
  if (p.doc) out.doc = p.doc;
  for (const [mid, m] of Object.entries(p.models ?? {})) {
    out.models[mid] = slimModel(m);
  }
  return out;
}

console.log(`Fetching ${sourceUrl} ...`);
const res = await fetch(sourceUrl, {
  headers: { "user-agent": "locus-model-catalog-refresh" },
});
if (!res.ok) {
  console.error(`Fetch failed: HTTP ${res.status}`);
  process.exit(1);
}
const raw = await res.json();

const providers = {};
for (const [pid, p] of Object.entries(raw)) {
  if (!p || typeof p !== "object" || !p.models) continue;
  providers[pid] = slimProvider(p);
}

const providerCount = Object.keys(providers).length;
const modelCount = Object.values(providers).reduce(
  (acc, p) => acc + Object.keys(p.models).length,
  0,
);
if (providerCount < 50 || modelCount < 1000) {
  console.error(
    `Sanity check failed: got ${providerCount} providers / ${modelCount} models — refusing to overwrite snapshot.`,
  );
  process.exit(1);
}

const catalog = {
  version: 1,
  fetched_at: new Date().toISOString(),
  providers,
};

if (existsSync(outPath)) {
  try {
    const prev = JSON.parse(gunzipSync(readFileSync(outPath)).toString("utf-8"));
    const prevModels = Object.values(prev.providers ?? {}).reduce(
      (acc, p) => acc + Object.keys(p.models ?? {}).length,
      0,
    );
    console.log(`Previous snapshot: ${prevModels} models (fetched_at ${prev.fetched_at})`);
  } catch {
    // Previous snapshot unreadable; overwrite it.
  }
}

const json = JSON.stringify(catalog);
const gz = gzipSync(Buffer.from(json, "utf-8"), { level: 9 });
mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, gz);

console.log(
  `Wrote ${outPath}\n  providers: ${providerCount}\n  models: ${modelCount}\n  raw: ${(json.length / 1024).toFixed(0)} KB, gzip: ${(gz.length / 1024).toFixed(0)} KB`,
);
