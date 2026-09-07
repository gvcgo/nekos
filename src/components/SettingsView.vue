<script setup lang="ts">
import { onMounted, ref } from "vue";
import {
  coreVersion,
  settingsGet,
  settingsSet,
  type Settings,
} from "../api";
import { applyLocale, fmt, useDict, type Dict } from "../i18n";
import RouteSettings from "./RouteSettings.vue";

const zhL = {
  title: "设置",
  portLabel: "代理监听端口（127.0.0.1 mixed）",
  modeLabel: "模式",
  modeGlobal: "全局代理",
  modeRule: "规则(绕过大陆)",
  modeDirect: "直连（不走代理）",
  logLevelLabel: "核心日志级别（下次启动内核生效）",
  logError: "仅错误",
  logWarn: "警告+错误",
  logInfo: "信息（调试用）",
  logDebug: "调试",
  langLabel: "语言",
  langZh: "中文",
  langEn: "English",
  closeTray: "关闭窗口时最小化到托盘",
  filterTitle: "开启后，导入/更新订阅与粘贴导入时会丢弃 server 为 IPv6 地址的节点",
  filterLabel: "过滤 IPv6 节点（立即保存，对之后导入/更新生效）",
  autoTitle: "应用运行期间每分钟检查；距上次成功更新超过间隔的订阅自动抓取更新",
  autoLabel: "自动更新订阅（应用运行期间）",
  autoStartTitle: "登录时自动启动 nekos（写入 XDG autostart 桌面项，指向当前程序）",
  autoStartLabel: "开机自启（登录时启动）",
  autoStartOn: "已开启开机自启",
  autoStartOff: "已关闭开机自启",
  intervalLabel: "更新间隔",
  intervalMin: "{n} 分钟",
  intervalH1: "1 小时",
  intervalH3: "3 小时",
  intervalH6: "6 小时",
  intervalH12: "12 小时",
  intervalH24: "24 小时",
  intervalSaved: "更新间隔设为 {n} 分钟",
  autoOn: "自动更新已开启：应用运行期间后台定时抓取",
  autoOff: "自动更新已关闭",
  filterOn: "已开启 IPv6 过滤（对之后导入/更新生效）",
  filterOff: "已关闭 IPv6 过滤",
  proxyNote: "系统代理独立开关在「分组」页工具栏：内核运行中可随时开启/关闭，退出自动还原。",
  saveBtn: "保存",
  savingBtn: "保存中…",
  saved: "已保存",
  portRange: "端口需在 1-65535 之间",
  coreVersionLabel: "核心版本",
} as const;
type DictKeys = keyof typeof zhL;
const enL: Record<DictKeys, string> = {
  title: "Settings",
  portLabel: "Proxy listen port (127.0.0.1 mixed)",
  modeLabel: "Mode",
  modeGlobal: "Global proxy",
  modeRule: "Rule (bypass mainland)",
  modeDirect: "Direct (no proxy)",
  logLevelLabel: "Core log level (applies on next core start)",
  logError: "Errors only",
  logWarn: "Warnings + errors",
  logInfo: "Info (for debugging)",
  logDebug: "Debug",
  langLabel: "Language",
  langZh: "中文",
  langEn: "English",
  closeTray: "Minimize to tray when closing window",
  filterTitle: "When enabled, nodes whose server is an IPv6 address are dropped when importing/updating subscriptions or pasting import lists",
  filterLabel: "Filter IPv6 nodes (saved immediately; applies to future imports/updates)",
  autoTitle: "While the app runs, checks every minute; subscriptions whose last successful update is older than the interval are fetched automatically",
  autoLabel: "Auto-update subscriptions (while the app runs)",
  autoStartTitle: "Launch nekos at login (writes an XDG autostart entry pointing at the current executable)",
  autoStartLabel: "Start at login",
  autoStartOn: "Start-at-login enabled",
  autoStartOff: "Start-at-login disabled",
  intervalLabel: "Update interval",
  intervalMin: "{n} min",
  intervalH1: "1 hour",
  intervalH3: "3 hours",
  intervalH6: "6 hours",
  intervalH12: "12 hours",
  intervalH24: "24 hours",
  intervalSaved: "Update interval set to {n} minutes",
  autoOn: "Auto-update enabled: subscriptions are fetched in the background while the app runs",
  autoOff: "Auto-update disabled",
  filterOn: "IPv6 filtering enabled (applies to future imports/updates)",
  filterOff: "IPv6 filtering disabled",
  proxyNote: "The separate system-proxy toggle lives in the toolbar of the Groups page: it can be turned on/off any time the core is running and resets automatically on exit.",
  saveBtn: "Save",
  savingBtn: "Saving…",
  saved: "Saved",
  portRange: "Port must be between 1 and 65535",
  coreVersionLabel: "Core version",
};
const dict = useDict({ zh: zhL as Dict, en: enL });
function tt(k: DictKeys, p?: Record<string, string | number>): string {
  return fmt(dict.value[k] as string, p);
}

const settings = ref<Settings | null>(null);
const version = ref("…");
const saving = ref(false);
const savedMsg = ref("");
const err = ref("");

const portStr = ref("2080");
const mode = ref("global");
const logLevel = ref("warn");
const closeToTray = ref(true);
const filterIpv6 = ref(false);
const autoUpdate = ref(false);
const autoMinutes = ref(360);
const autoStart = ref(false);
const language = ref<"zh" | "en">("zh");
const minuteOptions = [15, 30, 60, 180, 360, 720, 1440];

function intervalLabel(m: number): string {
  if (m < 60) return tt("intervalMin", { n: m });
  switch (m) {
    case 60:
      return tt("intervalH1");
    case 180:
      return tt("intervalH3");
    case 360:
      return tt("intervalH6");
    case 720:
      return tt("intervalH12");
    default:
      return tt("intervalH24");
  }
}

onMounted(async () => {
  try {
    settings.value = await settingsGet();
    portStr.value = String(settings.value.port);
    mode.value = settings.value.mode;
    logLevel.value = settings.value.log_level ?? "warn";
    closeToTray.value = settings.value.close_to_tray;
    filterIpv6.value = settings.value.filter_ipv6 ?? false;
    autoUpdate.value = settings.value.auto_update_subscriptions ?? false;
    autoMinutes.value = settings.value.auto_update_minutes ?? 360;
    autoStart.value = settings.value.auto_start ?? false;
    const l = settings.value.language;
    language.value = l === "en" ? l : "zh";
    applyLocale(language.value);
    version.value = await coreVersion();
  } catch (e) {
    err.value = String(e);
  }
});

async function toggleAutoUpdate(on: boolean) {
  try {
    settings.value = await settingsSet({ auto_update_subscriptions: on });
    autoUpdate.value = on;
    savedMsg.value = on ? tt("autoOn") : tt("autoOff");
  } catch (e) {
    err.value = String(e);
  }
}

async function setAutoMinutes(minutes: number) {
  try {
    settings.value = await settingsSet({ auto_update_minutes: minutes });
    autoMinutes.value = minutes;
    savedMsg.value = tt("intervalSaved", { n: minutes });
  } catch (e) {
    err.value = String(e);
  }
}

async function toggleFilter(on: boolean) {
  try {
    settings.value = await settingsSet({ filter_ipv6: on });
    filterIpv6.value = on;
    savedMsg.value = on ? tt("filterOn") : tt("filterOff");
  } catch (e) {
    err.value = String(e);
  }
}

async function toggleAutoStart(on: boolean) {
  try {
    settings.value = await settingsSet({ auto_start: on });
    autoStart.value = on;
    savedMsg.value = on ? tt("autoStartOn") : tt("autoStartOff");
  } catch (e) {
    err.value = String(e);
  }
}

async function setLanguage(v: "zh" | "en") {
  try {
    settings.value = await settingsSet({ language: v });
    language.value = v;
    applyLocale(v);
    savedMsg.value = tt("saved");
  } catch (e) {
    err.value = String(e);
  }
}

async function save() {
  saving.value = true;
  savedMsg.value = "";
  err.value = "";
  try {
    const port = parseInt(portStr.value, 10);
    if (!(port >= 1 && port <= 65535)) {
      err.value = tt("portRange");
      return;
    }
    settings.value = await settingsSet({
      port,
      mode: mode.value,
      log_level: logLevel.value,
      close_to_tray: closeToTray.value,
      filter_ipv6: filterIpv6.value,
    });
    savedMsg.value = tt("saved");
  } catch (e) {
    err.value = String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="settings-page">
    <h1>{{ tt("title") }}</h1>
    <div class="card">
      <label>{{ tt("portLabel") }}</label>
      <input v-model="portStr" class="field" type="number" min="1" max="65535" />

      <label>{{ tt("modeLabel") }}</label>
      <select v-model="mode" class="field">
        <option value="global">{{ tt("modeGlobal") }}</option>
        <option value="rule">{{ tt("modeRule") }}</option>
        <option value="direct">{{ tt("modeDirect") }}</option>
      </select>

      <label>{{ tt("logLevelLabel") }}</label>
      <select v-model="logLevel" class="field">
        <option value="error">{{ tt("logError") }}</option>
        <option value="warn">{{ tt("logWarn") }}</option>
        <option value="info">{{ tt("logInfo") }}</option>
        <option value="debug">{{ tt("logDebug") }}</option>
      </select>

      <label>{{ tt("langLabel") }}</label>
      <select :value="language" class="field" @change="setLanguage(($event.target as HTMLSelectElement).value as 'zh' | 'en')">
        <option value="zh">{{ tt("langZh") }}</option>
        <option value="en">{{ tt("langEn") }}</option>
      </select>

      <label class="check">
        <input v-model="closeToTray" type="checkbox" />
        {{ tt("closeTray") }}
      </label>
      <label class="check" :title="tt('filterTitle')">
        <input type="checkbox" :checked="filterIpv6" @change="toggleFilter(($event.target as HTMLInputElement).checked)" />
        {{ tt("filterLabel") }}
      </label>
      <label class="check" :title="tt('autoTitle')">
        <input type="checkbox" :checked="autoUpdate" @change="toggleAutoUpdate(($event.target as HTMLInputElement).checked)" />
        {{ tt("autoLabel") }}
      </label>
      <label class="check" :title="tt('autoStartTitle')">
        <input type="checkbox" :checked="autoStart" @change="toggleAutoStart(($event.target as HTMLInputElement).checked)" />
        {{ tt("autoStartLabel") }}
      </label>
      <div v-if="autoUpdate" class="auto-row">
        <label>{{ tt("intervalLabel") }}</label>
        <select :value="autoMinutes" class="field sel-sm" @change="setAutoMinutes(Number(($event.target as HTMLSelectElement).value))">
          <option v-for="m in minuteOptions" :key="m" :value="m">{{ intervalLabel(m) }}</option>
        </select>
      </div>
      <p class="note">{{ tt("proxyNote") }}</p>

      <div class="row">
        <button :disabled="saving" @click="save">{{ saving ? tt("savingBtn") : tt("saveBtn") }}</button>
        <span v-if="savedMsg" class="ok">{{ savedMsg }}</span>
        <span v-if="err" class="err">{{ err }}</span>
      </div>
    </div>

    <RouteSettings />

    <div class="card about">
      <span>{{ tt("coreVersionLabel") }}</span>
      <code class="mono">{{ version }}</code>
    </div>
  </div>
</template>

<style scoped>
.settings-page { max-width: 460px; display: flex; flex-direction: column; gap: 14px; }
h1 { margin: 0; font-size: 20px; }
.card {
  border: 1px solid var(--border); border-radius: 10px; padding: 14px;
  display: flex; flex-direction: column; gap: 8px; font-size: 13px;
}
.card label { margin-top: 6px; }
.field {
  border: 1px solid var(--border); border-radius: 6px; padding: 6px 8px;
  background: transparent; color: inherit; font-size: 13px; max-width: 220px;
}
.check { display: inline-flex; align-items: center; gap: 6px; cursor: pointer; margin-top: 2px; }
.auto-row { display: flex; align-items: center; gap: 10px; margin: 4px 0; }
.auto-row label { margin: 0; font-size: 12px; opacity: 0.85; width: 70px; }
.sel-sm { max-width: 130px; }
.row { display: flex; align-items: center; gap: 10px; margin-top: 10px; }
button {
  border: 0; border-radius: 6px; background: var(--accent); color: #fff;
  padding: 7px 14px; font-size: 13px; cursor: pointer;
}
button:disabled { opacity: 0.5; cursor: default; }
.ok { color: #22c55e; font-size: 12px; }
.err { color: #ef4444; font-size: 12px; }
.note { font-size: 12px; opacity: 0.65; margin: 4px 0 0; line-height: 1.5; }
.about { flex-direction: row; justify-content: space-between; align-items: center; }
.mono { font-family: ui-monospace, monospace; font-size: 12px; }
</style>
