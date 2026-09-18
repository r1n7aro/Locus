import { writeFile } from "node:fs/promises";
import { connect } from "./cdp";
const [browserUrl, targetId] = process.argv.slice(2);
const { client: main } = await connect(browserUrl!, targetId!);
const records: unknown[] = [];
let child: Awaited<ReturnType<typeof connect>>["client"] | null = null;
try {
  await main.evaluate(`(async()=>{
    await probe.sample();
    window.testPopupUrl='about:blank#locus-shared-workbench-workbench-univer-validation-'+Date.now();
    window.testPopup=window.open(testPopupUrl,'','width=1000,height=700');
    if (!testPopup) throw new Error('Shared native window was denied');
  })()`);
  const popupUrl=await main.evaluate<string>("testPopupUrl");
  let target: {id:string}|undefined;
  for(let attempt=0;attempt<40&&!target;attempt++) {
    target=(await (await fetch(`${browserUrl}/json/list`)).json()).find((t:any)=>t.url===popupUrl);
    if (!target) await new Promise(r=>setTimeout(r,100));
  }
  if(!target) throw new Error("No shared WebView2 target");
  child=(await connect(browserUrl!,target.id)).client;
  await main.evaluate(`(() => {
    const d=testPopup.document;d.title='Univer shared-window validation';
    for(const style of document.head.querySelectorAll('style,link[rel="stylesheet"]')) d.head.appendChild(style.cloneNode(true));
    const host=document.querySelector('#grid');d.body.appendChild(host);host.style.inset='0';
  })()`);
  await new Promise(r=>setTimeout(r,250));
  await child.send("Page.bringToFront");
  await child.evaluate("window.keyEvidence=[];window.addEventListener('keydown',e=>keyEvidence.push({key:e.key,trusted:e.isTrusted,target:e.target.tagName}),true)");
  const p:any=await child.evaluate(`(()=>{const r=opener.probe.sheet.getRange('A3').getCellRect();const b=document.querySelector('canvas[id^="univer-sheet-main-canvas"]').getBoundingClientRect();return {x:b.x+r.x+30,y:b.y+r.y+14}})()`);
  await child.send("Input.dispatchMouseEvent",{type:"mousePressed",...p,button:"left",buttons:1,clickCount:1});
  await child.send("Input.dispatchMouseEvent",{type:"mouseReleased",...p,button:"left",buttons:0,clickCount:1});
  const sendKey=async(key:string,code:string,n:number)=>{
    await child!.send("Input.dispatchKeyEvent",{type:"keyDown",key,code,windowsVirtualKeyCode:n});
    await child!.send("Input.dispatchKeyEvent",{type:"keyUp",key,code,windowsVirtualKeyCode:n});
  };
  await sendKey("F2","F2",113);
  await new Promise(r=>setTimeout(r,80));
  await child.send("Input.insertText",{text:"shared-window-edit"});
  await sendKey("Enter","Enter",13);
  await new Promise(r=>setTimeout(r,150));
  const actual=await main.evaluate("({value:probe.text('A3'),selection:probe.selection()})");
  records.push({name:"move existing Univer DOM into Locus shared native window and edit",passed:(actual as any).value==='shared-window-edit',actual,keys:await child.evaluate("keyEvidence")});
  const screenshot=await child.send("Page.captureScreenshot",{format:"png"}) as {data:string};
  await writeFile(new URL("results/shared-window.png",import.meta.url),Buffer.from(screenshot.data,"base64"));
  await main.evaluate(`(async()=>{
    await probe.workbook.endEditingAsync(false);
    const host=testPopup.document.querySelector('#grid');document.body.appendChild(host);host.style.inset='';
  })()`);
  const snapshot = await main.evaluate("probe.snapshot()");
  const source = await main.evaluate("probe.source");
  const hostUrl = await main.evaluate<string>("location.origin");
  await child.send("Page.navigate",{url:`${hostUrl}/?detached=1`});
  const deadline=Date.now()+15000;
  while(Date.now()<deadline) {
    try {if(await child.evaluate("window.probeReady"))break;}catch{}
    await new Promise(r=>setTimeout(r,100));
  }
  await child.evaluate(`probe.loadSnapshot(${JSON.stringify(snapshot)},${JSON.stringify(source)})`);
  const q:any=await child.evaluate(`(()=>{const r=probe.sheet.getRange('A3').getCellRect();const b=document.querySelector('canvas[id^="univer-sheet-main-canvas"]').getBoundingClientRect();return {x:b.x+r.x+30,y:b.y+r.y+14}})()`);
  await child.send("Input.dispatchMouseEvent",{type:"mousePressed",...q,button:"left",buttons:1,clickCount:1});
  await child.send("Input.dispatchMouseEvent",{type:"mouseReleased",...q,button:"left",buttons:0,clickCount:1});
  await sendKey("F2","F2",113);await new Promise(r=>setTimeout(r,80));
  await child.send("Input.insertText",{text:"standalone-window-edit"});
  await sendKey("Enter","Enter",13);await new Promise(r=>setTimeout(r,150));
  const standalone=await child.evaluate("({value:probe.text('A3'),selection:probe.selection(),frozen:probe.snapshot().sheets.sheet.freeze})");
  records.push({name:"new window own JavaScript runtime with transferred snapshot",passed:(standalone as any).value==='2:0standalone-window-edit',actual:standalone});
  const standaloneScreenshot=await child.send("Page.captureScreenshot",{format:"png"}) as {data:string};
  await writeFile(new URL("results/standalone-window.png",import.meta.url),Buffer.from(standaloneScreenshot.data,"base64"));
  console.log(JSON.stringify(records,null,2));
} catch(error) {
  records.push({name:"shared window test",passed:false,error:String(error)});
  console.log(String(error));
} finally {
  await main.evaluate(`(()=>{if(window.testPopup&&!testPopup.closed){if(!document.querySelector('#grid')){const host=testPopup.document.querySelector('#grid');if(host){document.body.appendChild(host);host.style.inset='';}}testPopup.close();}delete window.testPopup;})()`).catch(()=>{});
  await writeFile(new URL("results/shared-window.json",import.meta.url),JSON.stringify(records,null,2));
  child?.close();main.close();
}
