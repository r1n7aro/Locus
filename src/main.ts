import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import "./assets/hljs-theme.css";
import "./styles/typography.css";
import { initDebugConsole } from "./services/debugConsole";
import { bootstrapLocale } from "./i18n";
import { getSystemLocale } from "./services/system";
import { installTauriDevtoolsHotkeys } from "./services/tauriRuntime";

void initDebugConsole();
installTauriDevtoolsHotkeys();

const app = createApp(App);
app.use(createPinia());
app.mount("#app");

async function syncSystemLocale() {
  try {
    bootstrapLocale(await getSystemLocale());
  } catch {
    bootstrapLocale(null);
  }
}

void syncSystemLocale();
