import { existsSync, readFileSync, readdirSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { gzipSync } from "node:zlib";

const packages: { name: string; version: string; license: string; evidence: string }[] = [];
function scan(directory: string) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.name.startsWith(".")) continue;
    const path = `${directory}/${entry.name}`;
    if (entry.name.startsWith("@")) { scan(path); continue; }
    if (!existsSync(`${path}/package.json`)) continue;
    const manifest = JSON.parse(readFileSync(`${path}/package.json`, "utf8"));
    let license = manifest.license;
    let evidence = "package.json";
    if (!license && existsSync(`${path}/LICENSE`)) {
      const text = readFileSync(`${path}/LICENSE`, "utf8");
      if (/Apache License\s+Version 2\.0/.test(text)) { license = "Apache-2.0"; evidence = "LICENSE (package.json omits license)"; }
    }
    packages.push({ name: manifest.name, version: manifest.version, license: license ?? "UNKNOWN", evidence });
    if (existsSync(`${path}/node_modules`)) scan(`${path}/node_modules`);
  }
}
scan("node_modules");
const assets = readdirSync("dist/assets").map(name => {
  const bytes = readFileSync(`dist/assets/${name}`);
  return { name, bytes: bytes.length, gzip: gzipSync(bytes).length };
});
const licenseCounts: Record<string, number> = {};
for (const pkg of packages) licenseCounts[pkg.license] = (licenseCounts[pkg.license] ?? 0) + 1;
const report = {
  packages, licenseCounts,
  proPackages: packages.filter(pkg => pkg.name.startsWith("@univerjs-pro/")),
  unresolvedLicenses: packages.filter(pkg => pkg.license === "UNKNOWN"),
  assets, assetBytes: assets.reduce((n, a) => n + a.bytes, 0), assetGzipBytes: assets.reduce((n, a) => n + a.gzip, 0),
};
await mkdir("results", { recursive: true });
await writeFile("results/dependencies-and-bundle.json", JSON.stringify(report, null, 2));
console.log(JSON.stringify({ ...report, packages: packages.length, assets: assets.length }, null, 2));
if (report.proPackages.length || report.unresolvedLicenses.length) process.exitCode = 1;
