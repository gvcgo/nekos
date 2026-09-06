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
const closeToTray = ref(true);

onMounted(async () => {
  try {
    settings.value = await settingsGet();
    portStr.value = String(settings.value.port);
    mode.value = settings.value.mode;
    closeToTray.value = settings.value.close_to_tray;
    version.value = await coreVersion();
  } catch (e) {
    err.value = String(e);
  }
});

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
      close_to_tray: closeToTray.value,
    });
    savedMsg.value = "已保存（重启代理后生效）";
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
        <option value="direct">直连（不走代理）</option>
      </select>

      <label class="check">
        <input v-model="closeToTray" type="checkbox" />
        关闭窗口时最小化到托盘
      </label>
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
