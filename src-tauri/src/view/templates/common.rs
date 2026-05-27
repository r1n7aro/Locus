pub(super) fn main_ts() -> String {
    r##"import { createApp } from "vue";
import App from "./App.vue";
import "./style.css";

createApp(App).mount("#app");
"##
    .to_string()
}

pub(super) fn store_ts() -> String {
    r#"import { reactive } from "vue";

export const viewState = reactive({
  dirty: false,
  status: "idle",
});
"#
    .to_string()
}
