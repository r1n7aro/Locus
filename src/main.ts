import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import "./assets/hljs-theme.css";
import "./styles/typography.css";
import { initDebugConsole } from "./services/debugConsole";
import { bootstrapLocale } from "./i18n";
import { getSystemLocale } from "./services/system";

void initDebugConsole();

async function bootstrapApp() {
  let systemLocale: string | null = null;
  try {
    systemLocale = await getSystemLocale();
  } catch {
    systemLocale = null;
  }

  bootstrapLocale(systemLocale);
  createApp(App).use(createPinia()).mount("#app");
}

void bootstrapApp();
