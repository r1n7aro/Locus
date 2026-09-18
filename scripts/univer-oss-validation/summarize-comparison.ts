import { readFileSync, readdirSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { gzipSync } from "node:zlib";

const files = readdirSync("results").filter(name => name === "comparison.json" || /^comparison-\d+x\d+\.json$/.test(name));
const runs = files.flatMap(name => JSON.parse(readFileSync(`results/${name}`, "utf8")).results);
const median = (values: number[]) => [...values].sort((a,b)=>a-b)[Math.floor(values.length/2)]!;
const percentile = (values:number[], p:number) => [...values].sort((a,b)=>a-b)[Math.ceil(values.length*p)-1]!;
const groups = new Map<string, any[]>();
for (const run of runs) {
  const key = `${run.engine}:${run.rows}:${run.columns}`;
  const group = groups.get(key) ?? []; group.push(run); groups.set(key,group);
}
const summary = [...groups.values()].map(group => {
  const first=group[0], scroll=group.flatMap(run=>run.scroll), edits=group.flatMap(run=>run.edits.map((edit:any)=>edit.totalMs));
  const assets=[...new Set<string>(first.resources.map((asset:any)=>asset.url))];
  let loadedBytes=0,loadedGzip=0;
  for(const asset of assets){const bytes=readFileSync(`dist/assets/${asset}`);loadedBytes+=bytes.length;loadedGzip+=gzipSync(bytes).length;}
  return {engine:first.engine,rows:first.rows,columns:first.columns,rounds:group.length,
    loadMedian:median(group.map(run=>run.load.totalMs)),loadMin:Math.min(...group.map(run=>run.load.totalMs)),loadMax:Math.max(...group.map(run=>run.load.totalMs)),
    editMedian:median(edits),scrollMedian:median(scroll),scrollP95:percentile(scroll,.95),
    heapMB:median(group.map(run=>run.loadedHeap.usedSize))/1e6,
    heapDeltaMB:median(group.map(run=>run.loadedHeap.usedSize-run.emptyHeap.usedSize))/1e6,
    dom:first.metrics.elements,visibleCells:first.metrics.cells,loadedBytes,loadedGzip,assets};
});
await writeFile("results/comparison-summary.json",JSON.stringify(summary,null,2));
console.log(JSON.stringify(summary.map(({assets,...result})=>result),null,2));
