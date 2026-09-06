<script setup lang="ts">
import { onMounted, ref } from "vue";
import { subscribe, type Group, type Node, type SubscribeResult } from "../api";

const emit = defineEmits<{ (e: "saved", groupId: number): void }>();

const url = ref("");
const result = ref<SubscribeResult | null>(null);
const busy = ref(false);
const error = ref("");
const ua = ref("clash-verge/v2.5.2");
const extraHeaders = ref("");
const saveName = ref("");
const saveBusy = ref(false);
const saveDone = ref<Group | null>(null);

function fmtBytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[u]}`;
}

function fmtExpire(secs: number): string {
  return new Date(secs * 1000).toLocaleString();
}

function buildHeaders(): Record<string, string> {
  const headers: Record<string, string> = {};
  if (ua.value.trim()) headers["User-Agent"] = ua.value.trim();
  for (const line of extraHeaders.value.split("\n")) {
    const t = line.trim();
    if (!t || t.startsWith("#")) continue;
    const idx = t.indexOf(":");
    if (idx > 0) headers[t.slice(0, idx).trim()] = t.slice(idx + 1).trim();
  }
  return headers;
}

async function doFetch() {
  busy.value = true;
  error.value = "";
  result.value = null;
  try {
    result.value = await subscribe(url.value.trim(), buildHeaders());
    saveName.value = suggestName(result.value.url);
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

function suggestName(subUrl: string): string {
  try {
    const u = new URL(subUrl);
    return u.hostname;
  } catch {
    return "新订阅";
  }
}

async function doSave() {
  saveBusy.value = true;
  saveDone.value = null;
  try {
    const out = await subscribe(url.value.trim(), buildHeaders(), saveName.value.trim());
    saveDone.value = { id: out.group_id ?? 0, name: saveName.value.trim() };
    if (out.group_id) emit("saved", out.group_id);
  } catch (e) {
    error.value = String(e);
  } finally {
    saveBusy.value = false;
  }
}

onMounted(() => {
  // nothing global needed; component is self-sufficient
});
</script>

<template>
  <div class="sub-page">
    <div class="toolbar">
      <h1>订阅</h1>
      <span class="hint">抓取订阅地址并解析节点；可带自定义请求头</span>
    </div>

    <div class="import">
      <div class="row">
        <input v-model="url" class="url-input" placeholder="https://example.com/xxxx/sub 或 ...?clash=2" @keydown.enter="doFetch" />
        <button :disabled="busy || !url.trim()" @click="doFetch">{{ busy ? "抓取中…" : "抓取解析" }}</button>
      </div>

      <details class="headers">
        <summary>请求头（部分订阅校验 User-Agent）</summary>
        <div class="header-grid">
          <label>User-Agent</label>
          <input v-model="ua" class="url-input" placeholder="如 clash-verge/v2.5.2" />
        </div>
        <label class="extra-label">额外请求头（每行 Key: Value）</label>
        <textarea v-model="extraHeaders" rows="2" class="extra-headers" placeholder="Referer: https://example.com&#10;Authorization: Bearer xxxx" />
      </details>

      <span v-if="error" class="err">{{ error }}</span>

      <div v-if="result" class="result">
        <div class="result-head">
          <strong>解析结果</strong>
          <span>节点 {{ result.nodes.length }} · 错误 {{ result.errors.length }}</span>
        </div>
        <div v-if="result.userinfo" class="userinfo">
          <span>流量 {{ fmtBytes((result.userinfo.upload || 0) + (result.userinfo.download || 0)) }} /
            {{ fmtBytes(result.userinfo.total || 0) }}</span>
          <span v-if="result.userinfo.expire">到期 {{ fmtExpire(result.userinfo.expire) }}</span>
        </div>
        <div v-if="result.nodes.length" class="save-row">
          <input v-model="saveName" class="url-input save-name" placeholder="保存为新分组名称" />
          <button :disabled="saveBusy || !saveName.trim()" @click="doSave">
            {{ saveBusy ? "保存中…" : "保存到新分组" }}
          </button>
          <span v-if="saveDone" class="ok">已保存到「{{ saveDone.name }}」</span>
        </div>
        <table>
          <thead>
            <tr><th>类型</th><th>备注</th><th>ID</th></tr>
          </thead>
          <tbody>
            <tr v-for="n in result.nodes.slice(0, 200)" :key="n.id">
              <td><code>{{ n.type }}</code></td>
              <td>{{ n.remark }}</td>
              <td class="mono">{{ n.id }}</td>
            </tr>
          </tbody>
        </table>
        <div v-if="result.nodes.length > 200" class="truncated">… 仅预览前 200 条，保存将入库全部 {{ result.nodes.length }} 条</div>
        <ul v-if="result.errors.length" class="errs">
          <li v-for="(e, i) in result.errors" :key="i">
            {{ e.reason }} — <span class="mono">{{ e.snippet }}</span>
          </li>
        </ul>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sub-page {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.toolbar {
  display: flex;
  align-items: baseline;
  gap: 14px;
}
.toolbar h1 { margin: 0; font-size: 20px; }
.hint { opacity: 0.6; font-size: 12px; }
.import { max-width: 780px; }
.row { display: flex; align-items: center; gap: 10px; }
button {
  border: 0; border-radius: 6px; background: var(--accent); color: #fff;
  padding: 7px 14px; font-size: 13px; cursor: pointer; flex: none;
}
button:disabled { opacity: 0.5; cursor: default; }
.url-input {
  flex: 1; min-width: 0;
  border: 1px solid var(--border); border-radius: 8px; padding: 8px;
  background: transparent; color: inherit;
  font-family: ui-monospace, monospace; font-size: 12px;
}
.save-name { max-width: 300px; }
.headers { margin: 10px 0; font-size: 12px; }
.headers summary { cursor: pointer; opacity: 0.8; }
.header-grid { display: flex; align-items: center; gap: 10px; margin: 8px 0; }
.header-grid label { width: 84px; flex: none; }
.extra-label { display: block; margin: 4px 0; }
.extra-headers { width: 100%; min-height: 40px; border: 1px solid var(--border); border-radius: 8px; padding: 6px; background: transparent; color: inherit; font-family: ui-monospace, monospace; font-size: 12px; }
.err { color: #ef4444; font-size: 12px; }
.ok { color: #22c55e; font-size: 12px; }
.result { margin-top: 12px; border: 1px solid var(--border); border-radius: 8px; overflow: hidden; }
.result-head { display: flex; justify-content: space-between; padding: 8px 10px; background: rgba(0,0,0,0.04); font-size: 12px; }
.userinfo { display: flex; gap: 18px; padding: 8px 10px; font-size: 12px; background: rgba(34,197,94,0.06); border-top: 1px solid var(--border); }
.save-row { display: flex; align-items: center; gap: 10px; padding: 10px; border-top: 1px solid var(--border); }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; padding: 6px 10px; border-top: 1px solid var(--border); }
th { font-size: 11px; text-transform: uppercase; opacity: 0.7; }
.mono { font-family: ui-monospace, monospace; font-size: 11px; }
.truncated { padding: 6px 10px; font-size: 11px; opacity: 0.6; }
.errs { margin: 0; padding: 8px 10px 8px 26px; font-size: 12px; color: #ef4444; }
</style>
