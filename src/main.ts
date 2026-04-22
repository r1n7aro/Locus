import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import "./assets/hljs-theme.css";
import "./styles/typography.css";
import { initDebugConsole } from "./services/debugConsole";

void initDebugConsole();
createApp(App).use(createPinia()).mount("#app");
