import { createApp } from "vue";
import { createPinia } from "pinia";
import "../assets/hljs-theme.css";
import "../styles/app-global.css";
import "../styles/typography.css";
import "../styles/asset-icons.css";
import "./mobile.css";
import { bootstrapLocale } from "../i18n";
import { initTheme } from "../composables/useTheme";
import { initFonts } from "../composables/useDisplaySettings";
import { useChatInputSettings } from "../composables/useChatInputSettings";
import { installLocusRuntime } from "../services/locusRuntime";
import { installRemoteClientTransport } from "../services/remoteClient";
import { ensureSessionAttentionSync } from "../services/sessionAttention";
import { connectAppBridge } from "./bridge";
import { startRemoteEventDispatch } from "./events";
import MobileApp from "./MobileApp.vue";

// Entry of the page the desktop serves to the Locus Remote app. It runs
// inside the app's WebView; every call reaches the desktop through the app,
// which holds the pinned TLS connection and the device token.
bootstrapLocale(null);
initTheme();
initFonts();

// The Enter key of a phone keyboard starts a new line; the button sends.
// (In memory only: the stored setting belongs to this WebView's origin.)
useChatInputSettings().state.submitMode = "mod-enter-send";

const bridge = await connectAppBridge();
installLocusRuntime(bridge.runtime);
installRemoteClientTransport(bridge.transport);
const stopDispatch = await startRemoteEventDispatch();
// Unread results are kept by the desktop for every client.
ensureSessionAttentionSync();

const app = createApp(MobileApp, { bridge });
app.use(createPinia());
app.mount("#app");

window.addEventListener("pagehide", () => {
  app.unmount();
  void stopDispatch();
  bridge.dispose();
}, { once: true });
