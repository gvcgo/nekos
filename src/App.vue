<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { coreStatus, coreVersion, ping, settingsGet } from "./api";
import { applyLocale, fmt, useDict, type Dict } from "./i18n";
import LogView from "./components/LogView.vue";
import ServersView from "./components/ServersView.vue";
import SubscriptionsView from "./components/SubscriptionsView.vue";
import SettingsView from "./components/SettingsView.vue";

const zhL = {
  navServers: "分组",
  navSubscriptions: "订阅",
  navLogs: "日志",
  navSettings: "设置",
  coreRunning: "core 运行中",
  coreStopped: "core 已停止",
  stopped: "已停止",
  running: "运行中",
  runningProxyOn: "运行中 · 代理开",
} as const;
type DictKeys = keyof typeof zhL;
const enL: Record<DictKeys, string> = {
  navServers: "Groups",
  navSubscriptions: "Subscriptions",
  navLogs: "Logs",
  navSettings: "Settings",
  coreRunning: "core running",
  coreStopped: "core stopped",
  stopped: "Stopped",
  running: "Running",
  runningProxyOn: "Running · proxy on",
};
const dict = useDict({ zh: zhL as Dict, en: enL });
function tt(k: DictKeys, p?: Record<string, string | number>): string {
  return fmt(dict.value[k] as string, p);
}

type Page = "servers" | "subscriptions" | "logs" | "settings";

const page = ref<Page>("servers");
const version = ref("");
const running = ref(false);
const proxyOn = ref(false);
const timer = ref<number | undefined>(undefined);

const pageDefs: { id: Page; key: DictKeys }[] = [
  { id: "servers", key: "navServers" },
  { id: "subscriptions", key: "navSubscriptions" },
  { id: "logs", key: "navLogs" },
  { id: "settings", key: "navSettings" },
];
const pages = computed<{ id: Page; label: string }[]>(() =>
  pageDefs.map((p) => ({ id: p.id, label: tt(p.key) })),
);

async function pollStatus() {
  try {
    const s = await coreStatus();
    running.value = s.running;
    proxyOn.value = s.proxy_enabled;
  } catch {
    /* backend not reachable yet */
  }
}

onMounted(async () => {
  try {
    const s = await settingsGet();
    applyLocale(s.language ?? "zh");
  } catch {
    /* ignore */
  }
  try {
    version.value = (await coreVersion()).split("\n")[0] ?? "";
  } catch (e) {
    version.value = String(e);
  }
  try {
    await ping();
  } catch {
    /* ignore */
  }
  await pollStatus();
  timer.value = window.setInterval(pollStatus, 3000);
});

onUnmounted(() => {
  if (timer.value) window.clearInterval(timer.value);
});
</script>

<template>
  <div class="shell">
    <aside class="side">
      <div class="brand">nekos</div>
      <nav>
        <button
          v-for="p in pages"
          :key="p.id"
          :class="{ active: page === p.id }"
          @click="page = p.id"
        >
          {{ p.label }}
        </button>
      </nav>
      <div class="side-foot">
        <span class="dot" :class="{ on: running }" :title="running ? tt('coreRunning') : tt('coreStopped')"></span>
        <span class="run-state">{{ running ? (proxyOn ? tt('runningProxyOn') : tt('running')) : tt('stopped') }}</span>
        <code>{{ version }}</code>
      </div>
    </aside>

    <main class="main">
      <ServersView v-if="page === 'servers'" @saved="page = 'servers'" />
      <SubscriptionsView v-else-if="page === 'subscriptions'" @saved="page = 'servers'" />
      <LogView v-else-if="page === 'logs'" />
      <SettingsView v-else />
    </main>
  </div>
</template>

<style scoped>
.shell {
  display: flex;
  height: 100%;
}

.side {
  width: 172px;
  flex: none;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border);
  padding: 12px 8px;
  background: rgba(0, 0, 0, 0.02);
}

@media (prefers-color-scheme: dark) {
  .side {
    border-right-color: var(--border-dark);
    background: rgba(255, 255, 255, 0.02);
  }
}

.brand {
  font-weight: 700;
  letter-spacing: 0.5px;
  padding: 4px 10px 14px;
}

nav {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

nav button {
  text-align: start;
  padding: 8px 10px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  font-size: 14px;
  color: inherit;
  cursor: pointer;
}

nav button.active {
  background: var(--accent);
  color: #fff;
}

.side-foot {
  margin-top: auto;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  padding: 6px 10px;
  font-size: 11px;
  opacity: 0.8;
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #6b7280;
  display: inline-block;
}

.dot.on {
  background: #22c55e;
}

code {
  font-size: 10px;
  overflow-wrap: anywhere;
}

.main {
  flex: 1;
  overflow: auto;
  padding: 20px 24px;
}
</style>
