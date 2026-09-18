import { strict as assert } from "node:assert";
import { mkdir, writeFile } from "node:fs/promises";
import { connect } from "./cdp";

const [browserUrl, targetId] = process.argv.slice(2);
const { client: cdp, version, target } = await connect(browserUrl!, targetId!);
if (!/^http:\/\/127\.0\.0\.1:1492[12]\//.test(target.url)) throw new Error("Navigate the isolated test page to the validation host first.");
await mkdir(new URL("results/", import.meta.url), { recursive: true });
const records: { name: string; passed: boolean; durationMs: number; detail?: unknown; error?: string }[] = [];
const consoleMessages: unknown[] = [];
cdp.subscribeEvents(({ method, params }) => {
  if (method === "Runtime.exceptionThrown" || method === "Log.entryAdded"
    || method === "Runtime.consoleAPICalled" && ["error", "warning"].includes(String(params.type))) {
    consoleMessages.push({ method, params });
  }
});
await cdp.send("Runtime.enable");
await cdp.send("Log.enable");
await cdp.send("Page.bringToFront");
const evaluate = <T = any>(expression: string) => cdp.evaluate<T>(expression);
const settle = () => evaluate("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))");
async function check(name: string, action: () => Promise<unknown>) {
  const start = performance.now();
  try {
    const detail = await action();
    records.push({ name, passed: true, durationMs: performance.now() - start, detail });
    console.log(`PASS ${name} ${JSON.stringify(detail ?? "")}`);
  } catch (error) {
    records.push({ name, passed: false, durationMs: performance.now() - start, error: String(error) });
    console.log(`FAIL ${name} ${String(error)}`);
  }
}
async function point(a1: string) {
  return evaluate<{ x: number; y: number }>(`(() => {
    const range=probe.sheet.getRange(${JSON.stringify(a1)}), cell=range.getCell(), r=range.getCellRect();
    const visible=probe.sheet.getVisibleRange(), freeze=probe.snapshot().sheets.sheet.freeze;
    const canvas=document.querySelector('canvas[id^="univer-sheet-main-canvas"]').getBoundingClientRect();
    // Facade getCellRect is in worksheet coordinates, not viewport coordinates.
    // All drag fixtures have fixed widths/heights, integral scroll and zoom=1.
    const scrollX=cell.actualColumn >= freeze.xSplit ? (visible.startColumn-freeze.xSplit)*140 : 0;
    const scrollY=cell.actualRow >= freeze.ySplit ? (visible.startRow-freeze.ySplit)*28 : 0;
    return {x:canvas.left+r.x+r.width/2-scrollX,y:canvas.top+r.y+r.height/2-scrollY}; })()`);
}
async function mouse(type: string, p: { x: number; y: number }, extra = {}) {
  await cdp.send("Input.dispatchMouseEvent", { type, ...p, ...extra });
}
async function click(a1: string) {
  const p = await point(a1);
  await mouse("mousePressed", p, { button: "left", clickCount: 1, buttons: 1 });
  await mouse("mouseReleased", p, { button: "left", clickCount: 1, buttons: 0 });
  await settle();
}
async function drag(from: string, to: string) {
  const start = await point(from), end = await point(to);
  await mouse("mouseMoved", start);
  await mouse("mousePressed", start, { button: "left", buttons: 1, clickCount: 1 });
  for (let i = 1; i <= 8; i++) await mouse("mouseMoved", {
    x: start.x + (end.x - start.x) * i / 8, y: start.y + (end.y - start.y) * i / 8,
  }, { button: "left", buttons: 1 });
  await mouse("mouseReleased", end, { button: "left", buttons: 0, clickCount: 1 });
  await settle();
  return evaluate("probe.selection()");
}
async function key(key: string, code: string, windowsVirtualKeyCode: number, modifiers = 0) {
  await cdp.send("Input.dispatchKeyEvent", { type: "keyDown", key, code, windowsVirtualKeyCode, modifiers });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode, modifiers });
  await settle();
}
async function screenshot(name: string) {
  const { data } = await cdp.send("Page.captureScreenshot", { format: "png" }) as { data: string };
  await writeFile(new URL(`results/${name}.png`, import.meta.url), Buffer.from(data, "base64"));
}
try {
  const deadline = Date.now() + 15000;
  while (!await evaluate("window.probeReady") && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 100));
  assert.equal(await evaluate("window.probeReady"), true);
  await check("mount OSS plugins in WebView2", () => evaluate("probe.sample()"));
  for (const frozen of [0, 1, 3]) {
    await check(`native drag across ${frozen} frozen columns after scrolling`, async () => {
      await evaluate(`probe.freeze(${frozen})`);
      await evaluate(`probe.scroll(0, ${frozen ? 10 : 0})`);
      const range = await drag("A2", frozen ? "L4" : "D4");
      assert.equal(range.startColumn, 0); assert.equal(range.endColumn, frozen ? 11 : 3);
      assert.equal(range.startRow, 1); assert.equal(range.endRow, 3);
      await screenshot(`frozen-${frozen}`);
      return range;
    });
  }
  await check("Shift keyboard selection crosses freeze boundary", async () => {
    await evaluate("probe.scroll(0, 3)"); await click("C2");
    await key("ArrowRight", "ArrowRight", 39, 8);
    const selected = await evaluate("probe.selection()");
    assert.equal(selected.startColumn, 2); assert.equal(selected.endColumn, 3);
    return selected;
  });
  await check("copy serialization includes offscreen cells across freeze boundary", async () => {
    const copied = await evaluate(`(() => {
      const html=probe.sheet.getRange('A2:L4').generateHTML();
      return [...new DOMParser().parseFromString(html,'text/html').querySelectorAll('tr')]
        .map(row=>[...row.querySelectorAll('td')].map(cell=>cell.textContent));
    })()`);
    assert.deepEqual(copied, Array.from({length:3},(_,r)=>Array.from({length:12},(_,col)=>`${r+1}:${col}`)));
    return { rows: copied.length, columns: copied[0].length };
  });
  await check("initial CSV strings retain types and lexical source", async () => {
    const source = '\ufeff"id",long,date,formula,note,empty\r\n001,9007199254740993,2026-09-16,=1+1,"line1\nline2",\r\n';
    await evaluate(`probe.load(${JSON.stringify(source)})`);
    assert.equal(await evaluate("probe.unchangedRoundTrip()"), source);
    const data = await evaluate("probe.cells('A2:F2')");
    assert.deepEqual(data[0].map((cell: any) => cell.v), ["001", "9007199254740993", "2026-09-16", "=1+1", "line1\nline2", ""]);
    assert.ok(data[0].every((cell: any) => cell.t === 1 && !cell.f));
    return { sourceUnchanged: true, strings: data[0].map((cell: any) => cell.v) };
  });
  await check("cell edit patches CSV without rewriting untouched lexical fields", async () => {
    const before = await evaluate<string>("probe.source");
    await evaluate("probe.literal('C2', '修改日期')");
    const after = await evaluate<string>("probe.commitCell(1,2)");
    assert.equal(after, before.replace("2026-09-16", "修改日期"));
    return { retainedBomQuotesCrlfAndTrailingEmpty: true };
  });
  await check("native typed leading zeros stay text", async () => {
    await evaluate("probe.select('A3')"); await click("A3");
    await key("F2", "F2", 113);
    await cdp.send("Input.insertText", { text: "000123" });
    await key("Enter", "Enter", 13);
    const cell = await evaluate("probe.cells('A3')[0][0]");
    assert.equal(cell.v, "000123"); assert.equal(cell.t, 1);
    return cell;
  });
  await check("native typed formula-like string stays literal", async () => {
    await click("D3"); await key("F2", "F2", 113);
    await cdp.send("Input.insertText", { text: "=1+1" });
    await key("Tab", "Tab", 9);
    const cell = await evaluate("probe.cells('D3')[0][0]");
    assert.equal(cell.v, "=1+1"); assert.ok(!cell.f);
    return cell;
  });
  await check("Chromium IME composition commits Chinese once", async () => {
    await click("B3"); await key("F2", "F2", 113);
    await cdp.send("Input.imeSetComposition", { text: "中文输", selectionStart: 3, selectionEnd: 3 });
    await cdp.send("Input.imeSetComposition", { text: "中文输入验证", selectionStart: 6, selectionEnd: 6 });
    await cdp.send("Input.insertText", { text: "中文输入验证" });
    await key("Enter", "Enter", 13);
    const cell = await evaluate("probe.cells('B3')[0][0]");
    assert.equal(cell.v, "中文输入验证");
    return { value: cell.v, method: "CDP composition; OS candidate window not exercised" };
  });
  await check("native multiline text editing", async () => {
    await click("E3"); await key("F2", "F2", 113);
    await cdp.send("Input.insertText", { text: "第一行" });
    await key("Enter", "Enter", 13, 1);
    await cdp.send("Input.insertText", { text: "第二行" });
    await key("Enter", "Enter", 13);
    const text = await evaluate("probe.text('E3')");
    assert.equal(text.replaceAll('\r\n','\n').replaceAll('\r','\n'), "第一行\n第二行"); return {text};
  });
  await check("browser paste event fills a rectangle preserving leading zeros", async () => {
    await click("A5");
    const result = await evaluate(`(async () => {
      const data = new DataTransfer(); data.setData('text/plain', ${JSON.stringify("007\t0008\n009\t0010")});
      (document.activeElement ?? document.body).dispatchEvent(new ClipboardEvent('paste', {bubbles:true,cancelable:true,clipboardData:data}));
      await new Promise(r=>setTimeout(r,150)); return probe.values('A5:B6');
    })()`);
    assert.deepEqual(result, [["007", "0008"], ["009", "0010"]]); return result;
  });
  await check("undo and redo a native cell edit", async () => {
    await click("C8"); await key("F2", "F2", 113);
    await cdp.send("Input.insertText", { text: "undo-check" }); await key("Enter", "Enter", 13);
    assert.equal(await evaluate("probe.values('C8')[0][0]"), "undo-check");
    await evaluate("probe.api.undo()");
    assert.notEqual(await evaluate("probe.values('C8')[0][0]"), "undo-check");
    await evaluate("probe.api.redo()");
    assert.equal(await evaluate("probe.values('C8')[0][0]"), "undo-check"); return true;
  });
  await check("merge crosses freeze boundary and remains selectable", async () => {
    await evaluate("probe.sample()");
    await evaluate("probe.merge('B2:D3')");
    assert.equal(await evaluate("probe.sheet.getRange('B2:D3').isMerged()"), true);
    await evaluate("probe.scroll(0, 4)");
    await click("B2");
    const selected = await evaluate("probe.selection()");
    assert.equal(selected.startColumn, 1); assert.equal(selected.endColumn, 3);
    await screenshot("merged-frozen"); return selected;
  });
  await check("native merge and unmerge preserve covered source values", async () => {
    await evaluate("probe.unmerge('B2:D3')");
    const actual = await evaluate("probe.values('B2:D3')");
    assert.deepEqual(actual, [["1:1", "1:2", "1:3"], ["2:1", "2:2", "2:3"]]);
    return actual;
  });
  await check("view-only merge mutation preserves covered values and edited anchor", async () => {
    await evaluate("probe.sample()");
    const original = await evaluate("probe.values('B2:D3')");
    assert.equal(await evaluate("probe.mergeView('B2:D3')"), true);
    assert.deepEqual(await evaluate("probe.values('B2:D3')"), original);
    await evaluate("probe.literal('B2', 'view-merge-edit')");
    await evaluate("probe.mergeView('B2:D3',true)");
    const actual = await evaluate("probe.values('B2:D3')");
    original[0][0] = "view-merge-edit";
    assert.deepEqual(actual, original); return actual;
  });
  await check("merge cannot mutate authoritative CSV when only an anchor edit is committed", async () => {
    await evaluate("probe.sample()"); const before = await evaluate<string>("probe.source");
    await evaluate("probe.merge('B2:D3')"); await evaluate("probe.literal('B2', 'merged-edit')");
    const after = await evaluate<string>("probe.commitCell(1,1)");
    assert.equal(after, before.replace("1:1,", "merged-edit,")); return true;
  });
  await check("hide and show a column while frozen and merged", async () => {
    await evaluate("void probe.sheet.hideColumns(2)"); await settle();
    assert.equal(await evaluate("probe.snapshot().sheets.sheet.columnData[2].hd"), 1);
    await evaluate("void probe.sheet.showColumns(2)"); await settle();
    assert.notEqual(await evaluate("probe.snapshot().sheets.sheet.columnData[2].hd"), 1);
    return await evaluate("probe.snapshot().sheets.sheet.mergeData");
  });
  await check("zoom and row height preserve selection", async () => {
    await evaluate("probe.select('F5:G6')");
    const before = await evaluate("probe.selection()");
    await evaluate("probe.zoom(1.25)"); await evaluate("void probe.sheet.setRowHeight(4, 64)");
    await settle(); assert.deepEqual(await evaluate("probe.selection()"), before);
    assert.equal(await evaluate("probe.sheet.getZoom()"), 1.25);
    await screenshot("zoom-row-height"); return true;
  });
  await check("pane resize and temporary hide retain editor state", async () => {
    const before = await evaluate("probe.selection()");
    const dimensions = await evaluate(`(async()=>{
      const host=document.querySelector('#grid');host.style.width='700px';
      await new Promise(r=>setTimeout(r,200));
      const width=document.querySelector('canvas[id^="univer-sheet-main-canvas"]').getBoundingClientRect().width;
      host.style.display='none';await new Promise(r=>setTimeout(r,80));
      host.style.display='';host.style.width='';await new Promise(r=>setTimeout(r,200));
      return {resized:width,restored:document.querySelector('canvas[id^="univer-sheet-main-canvas"]').getBoundingClientRect().width};
    })()`);
    assert.equal(dimensions.resized, 700); assert.ok(dimensions.restored > 700);
    assert.deepEqual(await evaluate("probe.selection()"), before); return dimensions;
  });
  await check("dispose and recreate restores workbook, freeze, zoom and selection", async () => {
    const before = await evaluate("({selection:probe.selection(),freeze:probe.snapshot().sheets.sheet.freeze,zoom:probe.sheet.getZoom(),merges:probe.snapshot().sheets.sheet.mergeData})");
    await evaluate("probe.recreate()");
    const after = await evaluate("({selection:probe.selection(),freeze:probe.snapshot().sheets.sheet.freeze,zoom:probe.sheet.getZoom(),merges:probe.snapshot().sheets.sheet.mergeData})");
    assert.deepEqual(after, before); return after;
  });
  for (const [rows, columns] of [[5000, 20], [100, 1000]]) {
    await check(`performance ${rows} rows x ${columns} columns`, async () => {
      const load = await evaluate(`probe.sample(${rows},${columns})`);
      const scroll = await evaluate(`(async () => {
        const durations=[]; for(let i=0;i<20;i++) {const start=performance.now();await probe.scroll(Math.floor((${rows}-30)*i/19),Math.floor((${columns}-10)*i/19));durations.push(performance.now()-start);}
        const start=performance.now();await probe.literal('B2','incremental');return {durations,editMs:performance.now()-start,metrics:probe.metrics(),visible:probe.sheet.getVisibleRange()};
      })()`);
      assert.equal(load.cells, 100000); assert.ok(scroll.metrics.elements < 1000);
      await screenshot(`performance-${rows}-${columns}`); return { ...load, ...scroll };
    });
  }
} finally {
  const report = { createdAt: new Date().toISOString(), version, target, records, consoleMessages,
    totals: { passed: records.filter(r => r.passed).length, failed: records.filter(r => !r.passed).length } };
  await writeFile(new URL("results/report.json", import.meta.url), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report.totals));
  process.exitCode = report.totals.failed ? 1 : 0;
  cdp.close();
}
