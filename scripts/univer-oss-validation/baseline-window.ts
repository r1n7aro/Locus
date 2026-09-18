import { writeFile } from "node:fs/promises";
import { connect } from "./cdp";
const [browserUrl,targetId,hostUrl='http://127.0.0.1:14922']=process.argv.slice(2);
if(!/^http:\/\/127\.0\.0\.1:1492[12]$/.test(hostUrl))throw new Error('Expected a local validation host');
const {client:main}=await connect(browserUrl!,targetId!);
let child: Awaited<ReturnType<typeof connect>>["client"]|undefined;
let result:unknown;
try {
  await main.send("Page.navigate",{url:hostUrl+"/baseline.html?benchmark=1&sharedWindow=1"});
  const deadline=Date.now()+15000;
  while(Date.now()<deadline){try{if(await main.evaluate("location.pathname==='/baseline.html'&&window.probeReady"))break;}catch{}await new Promise(r=>setTimeout(r,25));}
  await main.evaluate(`(async()=>{await probe.sample(100,20);window.baselineTable=probe.table;window.testPopupUrl='about:blank#locus-shared-workbench-workbench-tabulator-validation-'+Date.now();window.testPopup=window.open(testPopupUrl,'','width=1120,height=760');if(!testPopup)throw new Error('Popup denied');})()`);
  const popupUrl=await main.evaluate<string>("testPopupUrl");
  let target:any;
  for(let i=0;i<40&&!target;i++){target=(await(await fetch(`${browserUrl}/json/list`)).json()).find((t:any)=>t.url===popupUrl);if(!target)await new Promise(r=>setTimeout(r,50));}
  if(!target)throw new Error('No child target');
  child=(await connect(browserUrl!,target.id)).client;
  await main.evaluate(`(()=>{const d=testPopup.document;d.title='Tabulator shared-window validation';for(const style of document.head.querySelectorAll('style,link[rel="stylesheet"]'))d.head.appendChild(style.cloneNode(true));const host=document.querySelector('#grid');d.body.appendChild(host);host.style.inset='0';})()`);
  await new Promise(r=>setTimeout(r,200));
  await child.send("Page.bringToFront");
  const p:any=await child.evaluate(`(()=>{const r=opener.baselineTable.getRow(2).getCell(opener.probe.view.columnOrder[0]).getElement().getBoundingClientRect();return{x:r.x+30,y:r.y+14}})()`);
  await child.send('Input.dispatchMouseEvent',{type:'mousePressed',...p,button:'left',buttons:1,clickCount:1});
  await child.send('Input.dispatchMouseEvent',{type:'mouseReleased',...p,button:'left',buttons:0,clickCount:1});
  const key=async(key:string,code:string,n:number)=>{await child!.send('Input.dispatchKeyEvent',{type:'keyDown',key,code,windowsVirtualKeyCode:n});await child!.send('Input.dispatchKeyEvent',{type:'keyUp',key,code,windowsVirtualKeyCode:n});};
  await key('F2','F2',113);await new Promise(r=>setTimeout(r,80));
  await child.send('Input.insertText',{text:'shared-window-edit'});await key('Enter','Enter',13);
  await new Promise(r=>setTimeout(r,150));
  const actual=await main.evaluate<any>("({value:baselineTable.getRow(2).getData()[probe.view.columnOrder[0]],snapshot:probe.grid.getSnapshot(),csvRow:probe.source.split('\\r\\n')[2]})");
  const points:any=await child.evaluate(`(()=>{const cell=(r,c)=>{const p=opener.baselineTable.getRow(r).getCell(opener.probe.view.columnOrder[c]).getElement().getBoundingClientRect();return{x:p.left+p.width/2,y:p.top+p.height/2}};return{start:cell(4,0),end:cell(6,5),after:cell(8,0)}})()`);
  await child.send('Input.dispatchMouseEvent',{type:'mousePressed',...points.start,button:'left',buttons:1,clickCount:1});
  await child.send('Input.dispatchMouseEvent',{type:'mouseMoved',...points.end,buttons:1});
  await child.send('Input.dispatchMouseEvent',{type:'mouseReleased',...points.end,button:'left',buttons:0,clickCount:1});
  await child.send('Input.dispatchMouseEvent',{type:'mouseMoved',...points.after,buttons:0});
  await new Promise(r=>setTimeout(r,80));
  const selection:any=await main.evaluate(`({snapshot:probe.grid.getSnapshot(),dragging:baselineTable.modules.selectRange.mousedown})`);
  const geometry:any=await child.evaluate(`(()=>{const r=document.querySelector('.csv-range-fragment[data-frozen="true"]').getBoundingClientRect();const c=opener.baselineTable.getRow(4).getCell(opener.probe.view.columnOrder[0]).getElement().getBoundingClientRect();return{rangeLeft:r.left,rangeTop:r.top,cellLeft:c.left,cellTop:c.top}})()`);
  result={name:'CsvGrid shared DOM move, edit and frozen range drag',passed:actual.value==='2:0shared-window-edit'&&!selection.dragging&&selection.snapshot.row===4&&selection.snapshot.endRow===6&&selection.snapshot.column===0&&selection.snapshot.endColumn===5&&Math.abs(geometry.rangeLeft-geometry.cellLeft)<1&&Math.abs(geometry.rangeTop-geometry.cellTop)<1,actual,selection,geometry};
  const shot=await child.send('Page.captureScreenshot',{format:'png'}) as {data:string};
  await writeFile(new URL('results/tabulator-shared-window.png',import.meta.url),Buffer.from(shot.data,'base64'));
}catch(error){result={passed:false,error:String(error)};}
finally{
  await main.evaluate(`(()=>{if(window.testPopup&&!testPopup.closed){if(!document.querySelector('#grid')){const host=testPopup.document.querySelector('#grid');if(host){document.body.appendChild(host);host.style.inset='';}}testPopup.close();}delete window.testPopup;})()`).catch(()=>{});
  await writeFile(new URL('results/tabulator-shared-window.json',import.meta.url),JSON.stringify(result,null,2));
  console.log(JSON.stringify(result,null,2));child?.close();main.close();
}
if(!(result as {passed?:boolean})?.passed)process.exitCode=1;
