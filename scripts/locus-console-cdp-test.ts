// Run against an isolated dev instance:
// bun run scripts/locus-console-cdp-test.ts --browser-url http://127.0.0.1:<port>
import assert from "node:assert/strict";
import { CdpClient, findLocusWebViewTarget, sleep } from "./locus-webview2-stress-client";

const urlIndex = process.argv.indexOf("--browser-url");
const browserUrl = urlIndex >= 0 ? process.argv[urlIndex + 1] : undefined;
assert(browserUrl, "Pass --browser-url for the isolated locus:test:app instance.");
await fetch(`${browserUrl}/json/version`).then((response) => response.json());
const target = await findLocusWebViewTarget(browserUrl, 10_000);
assert(/^http:\/\/(localhost|127\.0\.0\.1):\d+\/$/.test(target.url), "Refusing to inject test logs into a published instance.");
const cdp = await CdpClient.connect(target.webSocketDebuggerUrl!);

type Metrics = {
  count: number;
  listHeight: number;
  listScrollHeight: number;
  outerHeight: number;
  outerScrollHeight: number;
  rowPositions: string[];
  maxGap: number;
  minGap: number;
  scrollTop: number;
  firstMessage: string;
};
const results: Array<{ scenario: string; metrics: Metrics }> = [];

async function checkLayout(scenario: string) {
  await sleep(180);
  const metrics = await cdp.evaluate<Metrics>(`(() => {
    const list = document.querySelector('.console-list');
    const outer = document.querySelector('.settings-content');
    const rows = [...document.querySelectorAll('.console-row')];
    const rects = rows.map(row => row.getBoundingClientRect());
    const gaps = rects.slice(1).map((rect, index) => rect.top - rects[index].bottom);
    return {
      count: rows.length, listHeight: list.clientHeight, listScrollHeight: list.scrollHeight,
      outerHeight: outer.clientHeight, outerScrollHeight: outer.scrollHeight,
      rowPositions: [...new Set(rows.map(row => getComputedStyle(row).position))],
      maxGap: Math.max(0, ...gaps), minGap: Math.min(0, ...gaps), scrollTop: list.scrollTop,
      firstMessage: rows[0]?.querySelector('.console-message')?.textContent?.slice(0, 80) ?? ''
    };
  })()`);
  assert(metrics.listHeight > 100, `${scenario}: viewport collapsed`);
  assert(metrics.outerScrollHeight <= metrics.outerHeight + 1, `${scenario}: logs expanded the settings page`);
  assert(metrics.count > 0 && metrics.count < 90, `${scenario}: unbounded or empty virtual window (${metrics.count})`);
  assert.deepEqual(metrics.rowPositions, ["absolute"], `${scenario}: virtual rows lost absolute positioning`);
  assert(metrics.maxGap <= 1.01 && metrics.minGap >= -0.01, `${scenario}: rows overlap or have gaps`);
  results.push({ scenario, metrics });
  return metrics;
}

try {
  await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false });
  await cdp.evaluate(`(() => {
    const state = { errors: [], longTasks: [], frames: 0, maxFrameGap: 0, previousFrame: performance.now() };
    state.onError = event => state.errors.push(event.message);
    state.onRejection = event => state.errors.push(String(event.reason));
    state.onFrame = now => {
      state.frames++;
      state.maxFrameGap = Math.max(state.maxFrameGap, now - state.previousFrame);
      state.previousFrame = now;
      state.frame = requestAnimationFrame(state.onFrame);
    };
    state.observer = new PerformanceObserver(list => state.longTasks.push(...list.getEntries().map(e => e.duration)));
    state.observer.observe({ type: 'longtask', buffered: false });
    window.addEventListener('error', state.onError);
    window.addEventListener('unhandledrejection', state.onRejection);
    state.frame = requestAnimationFrame(state.onFrame);
    window.__locusConsoleCdpTest = state;
  })()`);
  await cdp.evaluate(`document.querySelectorAll('button.tab-item')[0].click()`);
  await cdp.evaluate(`document.querySelectorAll('button.tab-item')[3].click()`);
  await cdp.evaluate(`Array.from(document.querySelectorAll('.settings-sidebar button')).find(e => ['控制台', 'Console'].includes(e.textContent.trim())).click()`);
  await cdp.evaluate(`for (let i = 0; i < 2000; i++) console.info('[console-cdp-test]', i, i % 7 === 0 ? 'multi line\\n'.repeat(10) : 'Sample log');`);
  await checkLayout("2000 mixed-height logs");

  await cdp.evaluate(`document.querySelector('.console-toggle button').click(); document.querySelector('.console-list').scrollTop = 20000;`);
  const manual = await checkLayout("manual scroll");
  await cdp.evaluate(`console.info('[console-cdp-test] newest marker');`);
  const afterAppend = await checkLayout("append while auto-scroll disabled");
  assert.equal(afterAppend.scrollTop, manual.scrollTop);

  await cdp.evaluate(`document.querySelector('.console-toggle button').click()`);
  const latest = await checkLayout("enable auto-scroll");
  assert.equal(latest.scrollTop, 0);
  assert(latest.firstMessage.includes("newest marker"));

  await cdp.evaluate(`console.info('[console-cdp-test]', 'long marker ' + 'x'.repeat(5000))`);
  await checkLayout("long log preview");
  await cdp.evaluate(`document.querySelector('.console-message-toggle').click()`);
  assert(await cdp.evaluate<boolean>(`document.querySelector('.console-message').textContent.length > 5000`));
  await checkLayout("expanded long log");
  await cdp.evaluate(`document.querySelector('.console-message-toggle').click()`);
  await checkLayout("collapsed long log");

  for (const [width, height] of [[960, 600], [1792, 1222], [1440, 900]]) {
    await cdp.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
    await checkLayout(`resize ${width}x${height}`);
  }
  await cdp.evaluate(`for (let i = 0; i < 12; i++) document.querySelectorAll('.console-column-handle')[2].dispatchEvent(new KeyboardEvent('keydown', {key:'ArrowRight', bubbles:true}));`);
  await checkLayout("resize module column");

  await cdp.evaluate(`document.querySelector('.console-toggle button').click(); document.querySelector('.console-list').scrollTop = 60000;`);
  await checkLayout("scroll older logs");
  await cdp.evaluate(`(() => { const input = document.querySelector('.console-search'); input.value = 'newest marker'; input.dispatchEvent(new Event('input', {bubbles:true})); })()`);
  const filtered = await checkLayout("filter while scrolled");
  assert.equal(filtered.scrollTop, 0);
  assert.equal(filtered.count, 1);
  await cdp.evaluate(`(() => { const input = document.querySelector('.console-search'); input.value = ''; input.dispatchEvent(new Event('input', {bubbles:true})); })()`);
  await checkLayout("clear filter");

  for (let iteration = 0; iteration < 6; iteration++) {
    await cdp.evaluate(`document.querySelectorAll('button.tab-item')[0].click()`);
    await cdp.evaluate(`document.querySelectorAll('button.tab-item')[3].click()`);
    await sleep(60);
  }
  await checkLayout("remount six times");

  await cdp.evaluate(`document.querySelector('.console-toggle button').getAttribute('aria-checked') === 'false' && document.querySelector('.console-toggle button').click()`);
  for (let batch = 0; batch < 20; batch++) {
    await cdp.evaluate(`for (let i = 0; i < 100; i++) console.info('[console-cdp-test] burst ${batch}', i, i % 5 === 0 ? 'wrapped log '.repeat(80) : 'short log');`);
    await sleep(50);
  }
  const burst = await checkLayout("2000 further logs in 20 bursts");
  assert.equal(burst.scrollTop, 0);
  assert(burst.firstMessage.includes("burst 19 99"));
  await cdp.send("Emulation.clearDeviceMetricsOverride");
  const nativeViewport = await checkLayout("restore native viewport");
  assert.equal(nativeViewport.scrollTop, 0);
  const idle = await cdp.evaluate<{ mutations: number }>(`new Promise(resolve => {
    let mutations = 0;
    const observer = new MutationObserver(records => { mutations += records.length; });
    observer.observe(document.querySelector('.console-virtual-body'), { childList:true, subtree:true, attributes:true });
    setTimeout(() => {observer.disconnect(); resolve({mutations});}, 1500);
  })`);
  const health = await cdp.evaluate<{ errors: string[]; longTasks: number[]; frames: number; maxFrameGap: number }>(`(() => {
    const { errors, longTasks, frames, maxFrameGap } = window.__locusConsoleCdpTest;
    return { errors, longTasks, frames, maxFrameGap };
  })()`);
  assert.deepEqual(health.errors, [], "Browser errors during console stress test");
  assert.equal(idle.mutations, 0, "Console kept updating after the log stream stopped");
  console.log(JSON.stringify({ passed: true, browserUrl, results, health, idle }, null, 2));
} finally {
  await cdp.evaluate(`(() => {
    const state = window.__locusConsoleCdpTest;
    if (!state) return;
    window.removeEventListener('error', state.onError);
    window.removeEventListener('unhandledrejection', state.onRejection);
    cancelAnimationFrame(state.frame);
    state.observer.disconnect();
    delete window.__locusConsoleCdpTest;
  })()`).catch(() => undefined);
  await cdp.send("Emulation.clearDeviceMetricsOverride").catch(() => undefined);
  cdp.close();
}
