import { strict as assert } from "node:assert";
import { mkdir, writeFile } from "node:fs/promises";
import { connect } from "./cdp";

const [browserUrl, targetId, requestedRows, requestedColumns] = process.argv.slice(2);
const cases = requestedRows && requestedColumns ? [[Number(requestedRows), Number(requestedColumns)]]
  : [[100,20],[5000,20],[1000,100],[100,1000]];
const { client: c, version } = await connect(browserUrl!, targetId!);
const results: any[] = [], runtimeIssues: any[] = [];
let context: any = {};
c.subscribeEvents(({method,params})=>{
  if(method==='Runtime.exceptionThrown'||method==='Runtime.consoleAPICalled'&&['error','warning'].includes(String(params.type))) {
    runtimeIssues.push({context:{...context},method,params});
  }
});
await c.send("Runtime.enable");
await c.send("Page.bringToFront");
await c.send("Emulation.setDeviceMetricsOverride",{width:1400,height:900,deviceScaleFactor:1,mobile:false});
await mkdir(new URL("results/",import.meta.url),{recursive:true});
const e=<T=any>(source:string)=>c.evaluate<T>(source);
async function navigate(engine:string, round:number, rows:number,columns:number) {
  const url=`http://127.0.0.1:14922/${engine==='tabulator'?'baseline.html':''}?benchmark=1&round=${round}&rows=${rows}&columns=${columns}`;
  await c.send("Page.navigate",{url});
  const deadline=Date.now()+30000;
  while(Date.now()<deadline){
    try { if(await e(`location.href===${JSON.stringify(url)}&&window.probeReady===true`))return; }catch{}
    await new Promise(r=>setTimeout(r,25));
  }
  throw new Error(`Timeout loading ${url}`);
}
try {
  for(const [rows,columns] of cases) {
    for(let round=0;round<3;round++) {
      for(const engine of round%2?['univer','tabulator']:['tabulator','univer']) {
        context={engine,rows,columns,round};
        await navigate(engine,round,rows!,columns!);
        await c.send("HeapProfiler.collectGarbage");
        const emptyHeap=await c.send("Runtime.getHeapUsage");
        const load=await e(`probe.sample(${rows},${columns})`);
        assert.equal(load.cells,rows!*columns!);
        const metrics=await e("probe.metrics()");
        assert.equal(metrics.width,1400);assert.equal(metrics.height,866);
        const scroll=await e(`(async()=>{const durations=[];for(let i=0;i<20;i++){
          const start=performance.now();await probe.scroll(Math.floor((${rows}-30)*i/19),Math.floor((${columns}-10)*i/19));durations.push(performance.now()-start);
        }return durations;})()`);
        await e("probe.scroll(0,2)");
        const edits=await e(`(async()=>{const values=[];for(let i=0;i<5;i++)values.push(await probe.benchmarkEdit(1,1,'edit-'+i));return values;})()`);
        await c.send("HeapProfiler.collectGarbage");
        const loadedHeap=await c.send("Runtime.getHeapUsage");
        const resources=await e("performance.getEntriesByType('resource').filter(r=>/\\.(js|css)(\\?|$)/.test(r.name)).map(r=>({url:r.name.split('/').pop(),bytes:r.decodedBodySize}))");
        const record={...context,load,scroll,edits,metrics,emptyHeap,loadedHeap,resources};
        results.push(record);
        console.log(JSON.stringify({engine,rows,columns,round,loadMs:load.totalMs,editMs:edits.map((r:any)=>r.totalMs),elements:metrics.elements,heap:(loadedHeap as any).usedSize}));
        if(round===0){const shot=await c.send('Page.captureScreenshot',{format:'png'})as{data:string};await writeFile(new URL(`results/compare-${engine}-${rows}-${columns}.png`,import.meta.url),Buffer.from(shot.data,'base64'));}
      }
    }
  }
} finally {
  const name = requestedRows && requestedColumns ? `comparison-${requestedRows}x${requestedColumns}.json` : 'comparison.json';
  await writeFile(new URL(`results/${name}`,import.meta.url),JSON.stringify({createdAt:new Date().toISOString(),version,results,runtimeIssues},null,2));
  await c.send('Emulation.clearDeviceMetricsOverride').catch(()=>{});
  c.close();
}
