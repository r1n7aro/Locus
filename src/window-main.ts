import { createApp } from "vue";
import { createPinia } from "pinia";
import WindowApp from "./WindowApp.vue";
import "./assets/hljs-theme.css";
// Design tokens (:root variables), scrollbars, and base element styles
// shared with the main entry; component scoped styles resolve against
// these variables in every window.
import "./styles/app-global.css";
import "./styles/typography.css";
import "./styles/asset-icons.css";
import { initDebugConsole } from "./services/debugConsole";
import { getSystemLocale } from "./services/system";
import {
  installTauriDevtoolsHotkeys,
  installTauriWindowDragFallback,
} from "./services/tauriRuntime";
import { markStartupPhase, scheduleStartupPaintReport } from "./services/startupPerf";

// Lightweight entry for every Locus sub-window (utility dialogs, View
// hosts, graph tools). It skips the main-app store/service graph so the
// window shell paints as fast as possible; the actual window component
// loads as an async chunk from WindowApp.
const debugConsoleReady = initDebugConsole();
markStartupPhase("frontend_window_main_enter", { href: window.location.href });
void debugConsoleReady.finally(() => {
  markStartupPhase("frontend_debug_console_ready");
});
installTauriDevtoolsHotkeys();
installTauriWindowDragFallback();

const app = createApp(WindowApp);
app.use(createPinia());
app.mount("#app");
markStartupPhase("frontend_window_vue_mount_called");
scheduleStartupPaintReport();
// Plugin inspector drawers register per window; the lightweight entry keeps
// loading them so diff, view-host, and value-editor windows expose the same
// inspector surface as the main window. Loaded dynamically: the module's
// static graph is heavy (~500KB) and must not block the shell's first paint.
void import("./services/inspectorDrawerExtensions")
  .then(({ bootstrapPluginInspectorDrawers }) => {
    bootstrapPluginInspectorDrawers();
  })
  .catch((error) => {
    console.warn("[sub-window] failed to load inspector drawer extensions:", error);
  });

// Loaded dynamically so the shell paints before any locale catalog parses;
// window components import `t` statically and therefore wait for the
// active catalog themselves (top-level await inside ./i18n).
async function syncSystemLocale() {
  markStartupPhase("frontend_locale_sync_start");
  const { bootstrapLocale } = await import("./i18n");
  try {
    bootstrapLocale(await getSystemLocale());
  } catch {
    bootstrapLocale(null);
  } finally {
    markStartupPhase("frontend_locale_sync_done");
  }
}

void syncSystemLocale();
