import { createApp } from "vue";
import App from "./App.vue";
import "./style.css";
import { installNativeCopyInterceptor } from "./clipboard";

installNativeCopyInterceptor();
createApp(App).mount("#app");
