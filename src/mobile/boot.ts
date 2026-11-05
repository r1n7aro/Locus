// Entry of mobile.html. The app is imported lazily so that a module that
// fails to load (flaky tunnel, desktop restarting) or a failed bootstrap
// leaves a way out instead of a blank page. Deliberately dependency-free.

function showBootFailure(error: unknown) {
  console.error("[remote] page failed to start", error);
  const zh = navigator.language.toLowerCase().startsWith("zh");
  const root = document.getElementById("app") ?? document.body;

  const box = document.createElement("main");
  box.style.cssText = "max-width:420px;margin:0 auto;padding:max(24px,env(safe-area-inset-top)) 24px 24px;"
    + "font:15px/1.5 system-ui,sans-serif";
  const title = document.createElement("h1");
  title.style.cssText = "font-size:18px;margin:24px 0 8px";
  title.textContent = zh ? "页面没有加载完整" : "The page did not load completely";
  const detail = document.createElement("p");
  detail.style.cssText = "margin:0 0 20px;opacity:.7;overflow-wrap:anywhere";
  detail.textContent = zh
    ? "与电脑的连接可能不稳定。可以重试，或返回设备列表。"
    : "The connection to the desktop may be unstable. Retry, or go back to the desktop list.";

  const actions = document.createElement("div");
  actions.style.cssText = "display:flex;gap:12px;flex-wrap:wrap";
  const retry = document.createElement("button");
  retry.type = "button";
  retry.style.cssText = "font:inherit;padding:8px 18px;border-radius:8px";
  retry.textContent = zh ? "重试" : "Retry";
  retry.addEventListener("click", () => location.reload());
  // Handled by the app's proxy, which shows the launcher again.
  const back = document.createElement("a");
  back.href = "/__locus_remote/leave";
  back.style.cssText = "padding:8px 4px";
  back.textContent = zh ? "返回设备列表" : "Back to desktops";
  actions.append(retry, back);

  box.append(title, detail, actions);
  root.replaceChildren(box);
}

import("./main").catch(showBootFailure);
