<script setup lang="ts">
import { onMounted, ref } from "vue";
import { coreVersion, parseText, ping, type ImportResult } from "./api";

type Page = "servers" | "subscriptions" | "settings";

const page = ref<Page>("servers");
const version = ref("…");
const bridge = ref("");
const importText = ref("");
const result = ref<ImportResult | null>(null);
const busy = ref(false);
const importError = ref("");

const pages: { id: Page; label: string }[] = [
  { id: "servers", label: "服务" },
  { id: "subscriptions", label: "订阅" },
  { id: "settings", label: "设置" },
];

onMounted(async () => {
  try {
    version.value = await coreVersion();
    bridge.value = await ping();
  } catch (e) {
    bridge.value = String(e);
  }
});

async function doParse() {
  busy.value = true;
  importError.value = "";
  try {
    result.value = await parseText(importText.value);
  } catch (e) {
    importError.value = String(e);
  } finally {
    busy.value = false;
  }
}
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
        <span class="dot" :title="bridge">core</span>
        <code>{{ version }}</code>
      </div>
    </aside>

    <main class="main">
      <section v-if="page === 'servers'" class="panel">
        <div class="panel-head">
          <h1>服务</h1>
          <span class="hint">P0 进行中 — 节点表/启停/系统代理随里程碑落地</span>
        </div>

        <div class="import">
          <h2>导入预览（校验 链接 → Rust → core 解析链路）</h2>
          <textarea
            v-model="importText"
            rows="6"
            placeholder="粘贴分享链接或订阅文本，如 anytls://… trojan://… vless://…"
          />
          <div class="row">
            <button :disabled="busy || !importText.trim()" @click="doParse">
              {{ busy ? "解析中…" : "解析预览" }}
            </button>
            <span v-if="importError" class="err">{{ importError }}</span>
          </div>

          <div v-if="result" class="result">
            <div class="result-head">
              <strong>解析结果</strong>
              <span>节点 {{ result.nodes.length }} · 错误 {{ result.errors.length }}</span>
            </div>
            <table v-if="result.nodes.length">
              <thead>
                <tr><th>类型</th><th>备注</th><th>ID</th></tr>
              </thead>
              <tbody>
                <tr v-for="n in result.nodes" :key="n.id">
                  <td><code>{{ n.type }}</code></td>
                  <td>{{ n.remark }}</td>
                  <td class="mono">{{ n.id }}</td>
                </tr>
              </tbody>
            </table>
            <ul v-if="result.errors.length" class="errs">
              <li v-for="(e, i) in result.errors" :key="i">
                L{{ e.line }}: {{ e.reason }} — <span class="mono">{{ e.snippet }}</span>
              </li>
            </ul>
          </div>
        </div>
      </section>

      <section v-else-if="page === 'subscriptions'" class="panel placeholder">
        P1：多订阅分组管理（自动更新/去重/userinfo）
      </section>

      <section v-else class="panel placeholder">
        P0：监听端口、模式、系统代理开关、日志（随编排层落地）
      </section>
    </main>
  </div>
</template>

<style scoped>
.shell {
  display: flex;
  height: 100%;
}

.side {
  width: 168px;
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
  text-align: left;
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
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  font-size: 11px;
  opacity: 0.75;
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #22c55e;
}

code {
  font-size: 11px;
  overflow-wrap: anywhere;
}

.main {
  flex: 1;
  overflow: auto;
  padding: 20px 24px;
}

.panel-head {
  display: flex;
  align-items: baseline;
  gap: 14px;
}

.hint {
  opacity: 0.6;
  font-size: 12px;
}

.import {
  margin-top: 16px;
  max-width: 760px;
}

.import h2 {
  font-size: 14px;
  font-weight: 600;
}

textarea {
  width: 100%;
  font-family: ui-monospace, "Cascadia Mono", monospace;
  font-size: 12px;
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 8px;
  background: transparent;
  color: inherit;
  resize: vertical;
}

.row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin: 10px 0;
}

button:not(nav button) {
  border: 0;
  border-radius: 6px;
  background: var(--accent);
  color: #fff;
  padding: 7px 14px;
  font-size: 13px;
  cursor: pointer;
}

button:disabled {
  opacity: 0.5;
  cursor: default;
}

.err {
  color: #ef4444;
  font-size: 12px;
}

.result {
  border: 1px solid var(--border);
  border-radius: 8px;
  overflow: hidden;
}

.result-head {
  display: flex;
  justify-content: space-between;
  padding: 8px 10px;
  background: rgba(0, 0, 0, 0.04);
  font-size: 12px;
}

table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}

th,
td {
  text-align: left;
  padding: 6px 10px;
  border-top: 1px solid var(--border);
}

th {
  font-size: 11px;
  text-transform: uppercase;
  opacity: 0.7;
}

.mono {
  font-family: ui-monospace, monospace;
  font-size: 11px;
}

.errs {
  margin: 0;
  padding: 8px 10px 8px 26px;
  font-size: 12px;
  color: #ef4444;
}

.placeholder {
  opacity: 0.55;
  font-size: 14px;
}
</style>
