import { invoke } from "@tauri-apps/api/core";
import "./launcher.css";

interface DesktopView {
  id: string;
  name: string;
  deviceName: string;
  pairedAtMs: number;
  lastAddress: string | null;
  addresses: string[];
}

type Screen = "list" | "code";

const zh = (navigator.language || "zh").toLowerCase().startsWith("zh");
const text = {
  title: "Locus Remote",
  subtitle: zh ? "在手机上查看和继续电脑上的 Locus 会话" : "Continue your Locus sessions from your phone",
  desktops: zh ? "已配对的电脑" : "Paired computers",
  empty: zh ? "还没有配对的电脑。在电脑上打开 Locus →设置→远程访问，点击「配对新设备」。" : "No paired computers yet. In Locus on your computer, open Settings → Remote Access and click “Pair a device”.",
  scan: zh ? "扫码配对" : "Scan to pair",
  enterCode: zh ? "输入配对码" : "Enter a pairing code",
  deviceName: zh ? "这台手机的名称" : "Name of this phone",
  address: zh ? "电脑地址（HTTPS）" : "Computer address (HTTPS)",
  code: zh ? "6 位配对码" : "6-digit code",
  codeHint: zh ? "仅适用于 tailscale serve 等提供有效证书的地址；局域网直连请扫码。" : "Only for addresses with a valid certificate, such as tailscale serve. Scan the QR code for direct LAN connections.",
  pair: zh ? "配对" : "Pair",
  back: zh ? "返回" : "Back",
  remove: zh ? "移除" : "Remove",
  removeConfirm: zh ? "移除「{0}」？这台手机将无法再连接它，除非重新配对。" : "Remove “{0}”? This phone will need to pair again to connect.",
  connecting: zh ? "正在连接…" : "Connecting…",
  removing: zh ? "正在移除…" : "Removing…",
  pairing: zh ? "正在配对，请在电脑上点击「允许」…" : "Pairing. Click “Allow” on the computer…",
  lastAddress: zh ? "上次连接：" : "Last used: ",
  cameraDenied: zh ? "需要相机权限才能扫码，请在系统设置中允许。" : "Camera access is needed to scan. Allow it in system settings.",
  scanUnavailable: zh ? "这台设备不支持扫码，请输入配对码。" : "Scanning is not available here. Enter a pairing code instead.",
};

function format(template: string, ...args: string[]): string {
  return template.replace(/\{(\d+)\}/g, (_, index) => args[Number(index)] ?? "");
}

function defaultDeviceName(): string {
  const saved = localStorage.getItem("locus-remote.deviceName");
  if (saved) return saved;
  // "…(Linux; Android 14; Pixel 8 Build/…)" → "Pixel 8"
  const match = /Android [\d.]+; ([^;)]+?)(?: Build|\))/.exec(navigator.userAgent);
  return match?.[1]?.trim() || (zh ? "我的手机" : "My phone");
}

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}

const state = {
  screen: "list" as Screen,
  desktops: [] as DesktopView[],
  busy: null as string | null,
  error: null as string | null,
};

const root = document.getElementById("app")!;

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: { class?: string; text?: string; attrs?: Record<string, string> } = {},
  children: (Node | null)[] = [],
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (props.class) node.className = props.class;
  if (props.text !== undefined) node.textContent = props.text;
  for (const [name, value] of Object.entries(props.attrs ?? {})) node.setAttribute(name, value);
  for (const child of children) if (child) node.append(child);
  return node;
}

function button(label: string, onClick: () => void, variant = "secondary", disabled = false): HTMLButtonElement {
  const node = el("button", { class: `btn btn-${variant}`, text: label, attrs: { type: "button" } });
  node.disabled = disabled;
  node.addEventListener("click", onClick);
  return node;
}

function render() {
  const children: (Node | null)[] = [
    el("header", { class: "top" }, [
      el("h1", { text: text.title }),
      el("p", { class: "muted", text: text.subtitle }),
    ]),
    state.error ? el("div", { class: "alert", text: state.error, attrs: { role: "alert" } }) : null,
    state.screen === "list" ? renderList() : renderCodeForm(),
    state.busy ? el("div", { class: "overlay" }, [el("div", { class: "spinner" }), el("p", { text: state.busy })]) : null,
  ];
  root.replaceChildren(...children.filter((child): child is Node => child !== null));
}

function renderList(): HTMLElement {
  const section = el("main", { class: "content" });
  section.append(el("h2", { text: text.desktops }));
  if (state.desktops.length === 0) {
    section.append(el("p", { class: "empty", text: text.empty }));
  } else {
    const list = el("ul", { class: "desktops" });
    for (const desktop of state.desktops) {
      const open = el("button", { class: "desktop", attrs: { type: "button" } }, [
        el("span", { class: "desktop-name", text: desktop.name }),
        el("span", { class: "muted small", text: desktop.lastAddress ? text.lastAddress + desktop.lastAddress : desktop.deviceName }),
      ]);
      open.addEventListener("click", () => void connect(desktop));
      const remove = button(text.remove, () => void forget(desktop), "ghost");
      list.append(el("li", { class: "desktop-row" }, [open, remove]));
    }
    section.append(list);
  }
  section.append(el("div", { class: "actions" }, [
    button(text.scan, () => void scanAndPair(), "primary"),
    button(text.enterCode, () => { state.screen = "code"; state.error = null; render(); }),
  ]));
  return section;
}

function field(label: string, input: HTMLInputElement): HTMLElement {
  return el("label", { class: "field" }, [el("span", { text: label }), input]);
}

function renderCodeForm(): HTMLElement {
  const form = el("form", { class: "content" });
  const address = el("input", { attrs: { type: "url", placeholder: "https://my-pc.tailnet.ts.net", autocapitalize: "off", spellcheck: "false", required: "" } });
  const code = el("input", { attrs: { inputmode: "numeric", maxlength: "7", placeholder: "123 456", autocomplete: "one-time-code", required: "" } });
  const name = el("input", { attrs: { maxlength: "64", required: "" } });
  name.value = defaultDeviceName();
  form.append(
    field(text.address, address),
    field(text.code, code),
    field(text.deviceName, name),
    el("p", { class: "muted small", text: text.codeHint }),
    el("div", { class: "actions" }, [
      button(text.back, () => { state.screen = "list"; state.error = null; render(); }),
      el("button", { class: "btn btn-primary", text: text.pair, attrs: { type: "submit" } }),
    ]),
  );
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void pairWithCode(address.value, code.value, name.value);
  });
  return form;
}

async function withBusy<T>(label: string, task: () => Promise<T>): Promise<T | undefined> {
  state.busy = label;
  state.error = null;
  render();
  try {
    return await task();
  } catch (error) {
    state.error = errorMessage(error);
    return undefined;
  } finally {
    state.busy = null;
    render();
  }
}

async function refresh() {
  try {
    state.desktops = await invoke<DesktopView[]>("launcher_list_desktops");
  } catch (error) {
    state.error = errorMessage(error);
  }
  render();
}

async function connect(desktop: DesktopView) {
  // On success the app navigates to the desktop's page.
  await withBusy(text.connecting, () => invoke("launcher_connect", { desktopId: desktop.id }));
}

async function forget(desktop: DesktopView) {
  if (!confirm(format(text.removeConfirm, desktop.name))) return;
  await withBusy(text.removing, () => invoke("launcher_forget", { desktopId: desktop.id }));
  await refresh();
}

async function afterPairing(desktop: DesktopView | undefined, deviceName: string) {
  if (!desktop) return;
  localStorage.setItem("locus-remote.deviceName", deviceName);
  state.screen = "list";
  await refresh();
  await connect(desktop);
}

async function scanAndPair() {
  let scanned: string | null = null;
  try {
    const scanner = await import("@tauri-apps/plugin-barcode-scanner");
    let permission = await scanner.checkPermissions();
    if (permission !== "granted") permission = await scanner.requestPermissions();
    if (permission !== "granted") {
      state.error = text.cameraDenied;
      render();
      return;
    }
    const result = await scanner.scan({ windowed: false, formats: [scanner.Format.QRCode] });
    scanned = result.content;
  } catch (error) {
    state.error = `${text.scanUnavailable} (${errorMessage(error)})`;
    render();
    return;
  }
  if (!scanned) return;
  const deviceName = defaultDeviceName();
  const desktop = await withBusy(text.pairing, () =>
    invoke<DesktopView>("launcher_pair_with_link", { link: scanned, deviceName }));
  await afterPairing(desktop, deviceName);
}

async function pairWithCode(url: string, code: string, deviceName: string) {
  const desktop = await withBusy(text.pairing, () =>
    invoke<DesktopView>("launcher_pair_with_code", { url, code, deviceName }));
  await afterPairing(desktop, deviceName);
}

render();
void refresh();
