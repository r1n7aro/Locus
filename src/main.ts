import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import "./assets/hljs-theme.css";
import "./styles/typography.css";
import "./styles/asset-icons.css";
import { initDebugConsole } from "./services/debugConsole";
import { bootstrapLocale } from "./i18n";
import { getSystemLocale } from "./services/system";
import {
  installTauriDevtoolsHotkeys,
  installTauriWindowDragFallback,
} from "./services/tauriRuntime";
import { markStartupPhase, scheduleStartupPaintReport } from "./services/startupPerf";

const debugConsoleReady = initDebugConsole();
markStartupPhase("frontend_main_enter", { href: window.location.href });
void debugConsoleReady.finally(() => {
  markStartupPhase("frontend_debug_console_ready");
});
installTauriDevtoolsHotkeys();
markStartupPhase("frontend_devtools_hotkeys_ready");
installTauriWindowDragFallback();
markStartupPhase("frontend_window_drag_fallback_ready");

const app = createApp(App);
markStartupPhase("frontend_vue_app_created");
app.use(createPinia());
markStartupPhase("frontend_pinia_ready");
app.mount("#app");
markStartupPhase("frontend_vue_mount_called");
scheduleStartupPaintReport();

async function syncSystemLocale() {
  markStartupPhase("frontend_locale_sync_start");
  try {
    bootstrapLocale(await getSystemLocale());
  } catch {
    bootstrapLocale(null);
  } finally {
    markStartupPhase("frontend_locale_sync_done");
  }
}

void syncSystemLocale();
