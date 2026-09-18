import { strict as assert } from "node:assert";
import { mkdir, writeFile } from "node:fs/promises";
import { connect } from "./cdp";

const [browserUrl, targetId, hostUrl = "http://127.0.0.1:14922"] = process.argv.slice(2);
if (!/^http:\/\/127\.0\.0\.1:1492[12]$/.test(hostUrl)) throw new Error("Expected a local validation host");
const { client: c, version } = await connect(browserUrl!, targetId!);
const results: Array<{ name: string; passed: boolean; actual?: unknown }> = [];
const issues: unknown[] = [];
c.subscribeEvents(({ method, params }) => {
  if (method === "Runtime.exceptionThrown" || method === "Runtime.consoleAPICalled" && ["warning", "error"].includes(String(params.type))) issues.push({ method, params });
});
const e = <T = any>(expression: string) => c.evaluate<T>(expression);
const settle = () => e("new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))");
async function snapshot(row: number, column: number, endRow = row, endColumn = column, scrollLeft = 1120) {
  await e(`probe.grid.applySnapshot({row:${row},column:${column},endRow:${endRow},endColumn:${endColumn},scrollLeft:${scrollLeft},scrollTop:0})`);
  await settle();
}
async function point(row: number, column: number) {
  return e<{ x: number; y: number }>(`(()=>{const r=probe.table.getRow(${row}).getCell(probe.view.columnOrder[${column}]).getElement().getBoundingClientRect();return{x:r.left+r.width/2,y:r.top+r.height/2}})()`);
}
async function drag(from: [number, number], to: [number, number]) {
  const a = await point(...from), b = await point(...to);
  await c.send("Input.dispatchMouseEvent", { type: "mousePressed", ...a, button: "left", buttons: 1, clickCount: 1 });
  for (let step = 1; step <= 8; step++) await c.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: a.x + (b.x - a.x) * step / 8, y: a.y + (b.y - a.y) * step / 8, buttons: 1 });
  await c.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...b, button: "left", buttons: 0, clickCount: 1 });
  await settle();
}
async function key(key: string, modifiers = 0) {
  const code = key, windowsVirtualKeyCode = ({ ArrowRight: 39, ArrowLeft: 37, ArrowDown: 40, Delete: 46, F2: 113, Enter: 13 })[key];
  await c.send("Input.dispatchKeyEvent", { type: "keyDown", key, code, windowsVirtualKeyCode, modifiers });
  await c.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode, modifiers });
  await settle();
}
async function check(name: string, action: () => Promise<unknown>) {
  const actual = await action(); results.push({ name, passed: true, actual }); console.log(`PASS ${name}`);
}
async function selection(rows: number[], columns: number[]) {
  const actual = await e("({rows:probe.grid.selectedRows(),columns:probe.grid.selectedColumns()})");
  assert.deepEqual(actual, { rows, columns }); return actual;
}
try {
  await c.send("Runtime.enable");
  await c.send("Page.bringToFront");
  await c.send("Emulation.setDeviceMetricsOverride", { width: 1400, height: 900, deviceScaleFactor: 1, mobile: false });
  const url = hostUrl + "/baseline.html?benchmark=1&frozenValidation=1";
  await c.send("Page.navigate", { url });
  let ready = false;
  for (let i = 0; i < 200 && !ready; i++) {
    try { ready = await e(`location.href===${JSON.stringify(url)}&&window.probeReady===true`); } catch {}
    if (!ready) await new Promise(r => setTimeout(r,50));
  }
  assert.ok(ready, "validation page ready");
  for (const frozen of [1, 3, 0]) {
    await e(String.raw`probe.load(Array.from({length:100},(_,r)=>Array.from({length:24},(_,c)=>r+':'+c).join(',')).join('\r\n'),${frozen})`);
    const startColumn = frozen ? 0 : 10;
    await snapshot(1, startColumn);
    await check(`${frozen} frozen: native mouse drag across scrolled columns`, async () => {
      await drag([1,startColumn],[3,12]);
      return selection([1,2,3],Array.from({length:13-startColumn},(_,i)=>i+startColumn));
    });
    await check(`${frozen} frozen: reverse drag and clipboard include virtualized cells`, async () => {
      await drag([3,12],[1,startColumn]);
      await selection([1,2,3],Array.from({length:13-startColumn},(_,i)=>i+startColumn));
      const copied = await e(`(()=>{const data=new DataTransfer();document.querySelector('.tabulator-tableholder').dispatchEvent(new ClipboardEvent('copy',{bubbles:true,cancelable:true,clipboardData:data}));return data.getData('text/plain')})()`);
      assert.equal(copied,[1,2,3].map(r=>Array.from({length:13-startColumn},(_,i)=>`${r}:${i+startColumn}`).join('\t')).join('\n'));
      return copied;
    });
    if (!frozen) continue;
    await check(`${frozen} frozen: split borders match cell bounds after horizontal and vertical scroll`, async () => {
      await snapshot(1,0,3,12);
      const geometry = await e(`(()=>{const parts=[...document.querySelectorAll('.csv-range-fragment')];const cell=(c)=>probe.table.getRow(1).getCell(probe.view.columnOrder[c]).getElement().getBoundingClientRect().toJSON();return{parts:parts.map(p=>({frozen:p.dataset.frozen,display:p.style.display,rect:p.getBoundingClientRect().toJSON(),clip:p.style.clipPath})),first:cell(0),edge:cell(${frozen-1}),last:cell(12)}})()`);
      const [fixed,scrolling] = geometry.parts;
      assert.equal(fixed.display,'block'); assert.equal(scrolling.display,'block');
      assert.ok(Math.abs(fixed.rect.left-geometry.first.left)<1);
      assert.ok(Math.abs(fixed.rect.right-geometry.edge.right)<1);
      assert.ok(Math.abs(scrolling.rect.right-geometry.last.right)<1);
      assert.ok(Math.abs(fixed.rect.top-geometry.first.top)<1);
      const layers = await e("({overlay:Number(getComputedStyle(document.querySelector('.tabulator-range-overlay')).zIndex),frozen:Number(getComputedStyle(document.querySelector('.tabulator-row .tabulator-frozen')).zIndex)})");
      assert.ok(layers.overlay > layers.frozen, "selection borders must paint above frozen cells");
      // Vertical virtualization must not leave a border over unrelated rows.
      await e("probe.scroll(70,10)"); await settle();
      assert.equal(await e("[...document.querySelectorAll('.csv-range-fragment')].every(p=>p.style.display==='none')"),true);
      await snapshot(1,0,3,12);
      return geometry;
    });
    await check(`${frozen} frozen: Shift+Right reveals the first scrolling column`, async () => {
      await snapshot(1,frozen-1); await e("probe.grid.focus()");
      await key("ArrowRight",8);
      await selection([1],[frozen-1,frozen]);
      assert.equal(await e("probe.grid.getSnapshot().scrollLeft"),0);
    });
    await check(`${frozen} frozen: Ctrl+Shift+Right uses the frozen boundary`, async () => {
      await snapshot(1,0); await e("probe.grid.focus()");
      await key("ArrowRight",10);
      await selection([1],Array.from({length:24},(_,i)=>i));
      const geometry = await e(`(()=>{const c=probe.table.getRow(1).getCell(probe.view.columnOrder[23]).getElement().getBoundingClientRect();const h=document.querySelector('.tabulator-tableholder');return{right:c.right,edge:h.getBoundingClientRect().left+h.clientWidth}})()`);
      assert.ok(geometry.right<=geometry.edge+1); return geometry;
    });
    await check(`${frozen} frozen: paste and delete preserve CSV coordinates and strings`, async () => {
      await snapshot(1,frozen-1,1,frozen-1,0);
      await e(String.raw`(()=>{const data=new DataTransfer();data.setData('text/plain','001\t9007199254740993\t=1+1\nx\ty\tz');document.querySelector('.tabulator-tableholder').dispatchEvent(new ClipboardEvent('paste',{bubbles:true,cancelable:true,clipboardData:data}))})()`); await settle();
      const values = await e(String.raw`probe.source.split('\r\n').slice(1,3).map(r=>r.split(',').slice(${frozen-1},${frozen+2}))`);
      assert.deepEqual(values,[['001','9007199254740993','=1+1'],['x','y','z']]);
      await snapshot(1,frozen-1,2,frozen+1,0); await e("probe.grid.focus()"); await key("Delete");
      assert.deepEqual(await e(String.raw`probe.source.split('\r\n').slice(1,3).map(r=>r.split(',').slice(${frozen-1},${frozen+2}))`),[['','',''],['','','']]);
      return values;
    });
    await check(`${frozen} frozen: hiding the first column keeps the next frozen columns on the left`, async () => {
      await e("probe.setView({columns:{...probe.view.columns,[probe.view.columnOrder[0]]:{...probe.view.columns[probe.view.columnOrder[0]],hidden:true}}})");
      await snapshot(1,1,3,12);
      const actual = await e(`probe.table.getColumns().filter(c=>c.isVisible()).slice(1,${frozen+1}).map(c=>({left:c.getElement().getBoundingClientRect().left,position:c.getElement().style.left,right:c.getElement().style.right}))`);
      assert.ok(actual.every((c:any)=>c.left>=40 && !c.right)); return actual;
    });
    await check(`${frozen} frozen: resizing and zooming keep borders aligned`, async () => {
      await e("probe.setView({columns:{...probe.view.columns,[probe.view.columnOrder[1]]:{...probe.view.columns[probe.view.columnOrder[1]],width:220}}})");
      await e("probe.grid.setZoom(1.5)"); await settle(); await settle();
      await snapshot(1,1,3,12);
      const actual = await e("(()=>{const r=document.querySelector('.csv-range-fragment[data-frozen=\"true\"]').getBoundingClientRect();const c=probe.table.getRow(1).getCell(probe.view.columnOrder[1]).getElement().getBoundingClientRect();return{rangeLeft:r.left,cellLeft:c.left,cellWidth:c.width}})()");
      assert.ok(Math.abs(actual.rangeLeft-actual.cellLeft)<1); assert.equal(actual.cellWidth,330); return actual;
    });
  }
  await check("no compatibility warnings or uncaught exceptions", async()=>{assert.deepEqual(issues,[]);return issues;});
  await e("probe.setView({frozenColumns:3})");
  await snapshot(1,0,3,12);
  await mkdir(new URL('results/',import.meta.url),{recursive:true});
  const shot=await c.send('Page.captureScreenshot',{format:'png'}) as {data:string};
  await writeFile(new URL('results/tabulator-frozen-selection.png',import.meta.url),Buffer.from(shot.data,'base64'));
} catch(error) {
  results.push({name:'validation stopped',passed:false,actual:String(error)});
  console.error(error); process.exitCode=1;
} finally {
  await mkdir(new URL('results/',import.meta.url),{recursive:true});
  await writeFile(new URL('results/tabulator-frozen-selection.json',import.meta.url),JSON.stringify({version,results,issues},null,2));
  await c.send('Emulation.clearDeviceMetricsOverride').catch(()=>{}); c.close();
}
