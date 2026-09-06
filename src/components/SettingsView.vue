<script setup lang="ts">
import { onMounted, ref } from "vue";
import {
  coreVersion,
  settingsGet,
  settingsSet,
  type Settings,
} from "../api";

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
const minuteOptions = [
  { m: 15, label: "15 分钟" },
  { m: 30, label: "30 分钟" },
  { m: 60, label: "1 小时" },
  { m: 180, label: "3 小时" },
  { m: 360, label: "6 小时" },
  { m: 720, label: "12 小时" },
  { m: 1440, label: "24 小时" },
];

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
    version.value = await coreVersion();
  } catch (e) {
    err.value = String(e);
  }
});

async function toggleAutoUpdate(on: boolean) {
  try {
    settings.value = await settingsSet({ auto_update_subscriptions: on });
    autoUpdate.value = on;
    savedMsg.value = on
      ? "自动更新已开启：应用运行期间后台定时抓取"
      : "自动更新已关闭";
  } catch (e) {
    err.value = String(e);
  }
}

async function setAutoMinutes(minutes: number) {
  try {
    settings.value = await settingsSet({ auto_update_minutes: minutes });
    autoMinutes.value = minutes;
    savedMsg.value = `更新间隔设为 ${minutes} 分钟`;
  } catch (e) {
    err.value = String(e);
  }
}

async function toggleFilter(on: boolean) {
  try {
    settings.value = await settingsSet({ filter_ipv6: on });
    filterIpv6.value = on;
    savedMsg.value = on ? "已开启 IPv6 过滤（对之后导入/更新生效）" : "已关闭 IPv6 过滤";
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
      err.value = "端口需在 1-65535 之间";
      return;
    }
    settings.value = await settingsSet({
      port,
      mode: mode.value,
      log_level: logLevel.value,
      close_to_tray: closeToTray.value,
      filter_ipv6: filterIpv6.value,
    });
    savedMsg.value = "已保存";
  } catch (e) {
    err.value = String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="settings-page">
    <h1>设置</h1>
    <div class="card">
      <label>代理监听端口（127.0.0.1 mixed）</label>
      <input v-model="portStr" class="field" type="number" min="1" max="65535" />

      <label>模式</label>
      <select v-model="mode" class="field">
        <option value="global">全局代理</option>
        <option value="rule">规则(绕过大陆)</option>
        <option value="direct">直连（不走代理）</option>
      </select>

      <label>核心日志级别（下次启动内核生效）</label>
      <select v-model="logLevel" class="field">
        <option value="error">仅错误</option>
        <option value="warn">警告+错误</option>
        <option value="info">信息（调试用）</option>
        <option value="debug">调试</option>
      </select>

      <label class="check">
        <input v-model="closeToTray" type="checkbox" />
        关闭窗口时最小化到托盘
      </label>
      <label class="check" title="开启后，导入/更新订阅与粘贴导入时会丢弃 server 为 IPv6 地址的节点">
        <input type="checkbox" :checked="filterIpv6" @change="toggleFilter(($event.target as HTMLInputElement).checked)" />
        过滤 IPv6 节点（立即保存，对之后导入/更新生效）
      </label>
      <label class="check" title="应用运行期间每分钟检查；距上次成功更新超过间隔的订阅自动抓取更新">
        <input type="checkbox" :checked="autoUpdate" @change="toggleAutoUpdate(($event.target as HTMLInputElement).checked)" />
        自动更新订阅（应用运行期间）
      </label>
      <div v-if="autoUpdate" class="auto-row">
        <label>更新间隔</label>
        <select :value="autoMinutes" class="field sel-sm" @change="setAutoMinutes(Number(($event.target as HTMLSelectElement).value))">
          <option v-for="opt in minuteOptions" :key="opt.m" :value="opt.m">{{ opt.label }}</option>
        </select>
      </div>
      <p class="note">系统代理独立开关在「服务」页工具栏：内核运行中可随时开启/关闭，退出自动还原。</p>

      <div class="row">
        <button :disabled="saving" @click="save">{{ saving ? "保存中…" : "保存" }}</button>
        <span v-if="savedMsg" class="ok">{{ savedMsg }}</span>
        <span v-if="err" class="err">{{ err }}</span>
      </div>
    </div>

    <div class="card about">
      <span>核心版本</span>
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
