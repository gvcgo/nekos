<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  createGroup,
  deleteGroup,
  groupsList,
  subscribe,
  subscriptionEdit,
  subscriptionRefresh,
  type Group,
  type SubUserInfo,
  type SubscribeResult,
} from "../api";

const emit = defineEmits<{ (e: "saved", groupId: number): void }>();

// ---- existing subscription list ----------------------------------------

const subs = ref<Group[]>([]);
const refreshing = ref<Record<number, boolean>>({});
const err = ref("");
const msg = ref("");

const subscriptions = computed(() =>
  subs.value.filter((g) => (g.sub_url ?? "").trim().length > 0),
);

async function loadSubs() {
  subs.value = await groupsList();
}

function userInfo(g: Group): SubUserInfo | null {
  if (!g.sub_userinfo) return null;
  try {
    return JSON.parse(g.sub_userinfo) as SubUserInfo;
  } catch {
    return null;
  }
}

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

async function delSub(g: Group) {
  if (!confirm(`删除订阅「${g.name}」及其节点？`)) return;
  err.value = "";
  try {
    await deleteGroup(g.id);
    await loadSubs();
  } catch (e) {
    err.value = String(e);
  }
}

async function refreshSub(g: Group) {
  refreshing.value[g.id] = true;
  err.value = "";
  msg.value = "";
  try {
    const out: SubscribeResult = await subscriptionRefresh(g.id);
    msg.value = `「${g.name}」已更新：${out.nodes.length} 节点，${out.errors.length} 错误`;
    await loadSubs();
  } catch (e) {
    err.value = String(e);
  } finally {
    refreshing.value[g.id] = false;
  }
}

// ---- inline editor ------------------------------------------------------

const editingId = ref<number | null>(null);
const editName = ref("");
const editUrl = ref("");
const editUA = ref("");
const editExtrasText = ref("");
const savingEdit = ref(false);

function extrasToText(json?: string): string {
  if (!json) return "";
  try {
    const map = JSON.parse(json) as Record<string, string>;
    return Object.entries(map)
      .map(([k, v]) => `${k}: ${v}`)
      .join("\n");
  } catch {
    return json;
  }
}

function extrasToJson(text: string): string {
  const map: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const t = line.trim();
    if (!t || t.startsWith("#")) continue;
    const idx = t.indexOf(":");
    if (idx > 0) map[t.slice(0, idx).trim()] = t.slice(idx + 1).trim();
  }
  return JSON.stringify(map);
}

function startEdit(g: Group) {
  editingId.value = g.id;
  editName.value = g.name;
  editUrl.value = g.sub_url ?? "";
  editUA.value = g.user_agent ?? "";
  editExtrasText.value = extrasToText(g.extra_headers);
  err.value = "";
}

function cancelEdit() {
  editingId.value = null;
}

async function saveEdit() {
  if (!editingId.value) return;
  savingEdit.value = true;
  err.value = "";
  msg.value = "";
  try {
    await subscriptionEdit(
      editingId.value,
      editName.value.trim(),
      editUrl.value.trim(),
      editUA.value,
      extrasToJson(editExtrasText.value),
    );
    msg.value = "已保存订阅配置（点「更新」按新配置重新抓取）";
    await loadSubs();
  } catch (e) {
    err.value = String(e);
  } finally {
    savingEdit.value = false;
  }
}

// ---- new subscription ---------------------------------------------------

const url = ref("");
const ua = ref("clash-verge/v2.5.2");
const extraHeaders = ref("");
const saveName = ref("");
const fetchBusy = ref(false);
const preview = ref<SubscribeResult | null>(null);

function buildHeaders(): Record<string, string> {
  const headers: Record<string, string> = {};
  if (ua.value.trim()) headers["User-Agent"] = ua.value.trim();
  const extras = extrasToJson(extraHeaders.value);
  if (extras !== "{}") Object.assign(headers, JSON.parse(extras));
  return headers;
}

function suggestName(subUrl: string): string {
  try {
    return new URL(subUrl).hostname;
  } catch {
    return "新订阅";
  }
}

async function doFetch() {
  fetchBusy.value = true;
  err.value = "";
  preview.value = null;
  try {
    preview.value = await subscribe(url.value.trim(), buildHeaders());
    if (!saveName.value.trim()) saveName.value = suggestName(preview.value.url);
  } catch (e) {
    err.value = String(e);
  } finally {
    fetchBusy.value = false;
  }
}

async function doSaveNew() {
  fetchBusy.value = true;
  err.value = "";
  try {
    const out = await subscribe(
      url.value.trim(),
      buildHeaders(),
      saveName.value.trim(),
    );
    if (out.group_id) {
      msg.value = `已创建订阅「${saveName.value.trim()}」（${out.nodes.length} 节点）`;
      preview.value = null;
      url.value = "";
      saveName.value = "";
      await loadSubs();
      emit("saved", out.group_id);
    }
  } catch (e) {
    err.value = String(e);
  } finally {
    fetchBusy.value = false;
  }
}

onMounted(loadSubs);
</script>

<template>
  <div class="sub-page">
    <div class="head">
      <h1>订阅</h1>
      <span class="hint">{{ subscriptions.length }} 个订阅组</span>
      <span class="spacer"></span>
      <span v-if="msg" class="ok">{{ msg }}</span>
      <span v-if="err" class="err">{{ err }}</span>
    </div>

    <!-- managed subscription list -->
    <section v-if="subscriptions.length" class="list">
      <table>
        <thead>
          <tr><th>名称</th><th>地址</th><th>流量/到期</th><th>更新于</th><th></th></tr>
        </thead>
        <tbody>
          <template v-for="g in subscriptions" :key="g.id">
            <tr>
              <td>{{ g.name }}</td>
              <td class="mono url" :title="g.sub_url">{{ g.sub_url }}</td>
              <td class="mono">
                <template v-if="userInfo(g)">
                  {{ fmtBytes((userInfo(g)!.upload || 0) + (userInfo(g)!.download || 0)) }} /
                  {{ fmtBytes(userInfo(g)!.total || 0)
                  }}<span v-if="userInfo(g)!.expire"> · {{ new Date(userInfo(g)!.expire! * 1000).toLocaleDateString() }}</span>
                </template>
                <span v-else class="dim">—</span>
              </td>
              <td class="mono">{{ g.updated_at ?? "—" }}</td>
              <td class="ops">
                <button class="ghost mini" @click="startEdit(g)">编辑</button>
                <button class="ghost mini" :disabled="refreshing[g.id]" @click="refreshSub(g)">
                  {{ refreshing[g.id] ? "更新中…" : "更新" }}
                </button>
                <button class="ghost mini" @click="emit('saved', g.id)">节点</button>
                <button class="ghost mini danger" @click="delSub(g)">删除</button>
              </td>
            </tr>

            <!-- inline editor -->
            <tr v-if="editingId === g.id" class="editor-row">
              <td colspan="5">
                <div class="editor">
                  <div class="grid">
                    <label>名称</label>
                    <input v-model="editName" class="inp" />
                    <label>订阅地址</label>
                    <input v-model="editUrl" class="inp mono" placeholder="https://example.com/sub 或 ...?clash=2" />
                    <label>User-Agent</label>
                    <input v-model="editUA" class="inp mono" placeholder="如 clash-verge/v2.5.2（留空则不发送）" />
                    <label>额外请求头</label>
                    <textarea v-model="editExtrasText" rows="2" class="inp mono" placeholder="Referer: https://example.com&#10;Authorization: Bearer xxx" />
                  </div>
                  <div class="btns">
                    <button :disabled="savingEdit" @click="saveEdit">{{ savingEdit ? "保存中…" : "保存修改" }}</button>
                    <button class="ghost" @click="cancelEdit">取消</button>
                  </div>
                </div>
              </td>
            </tr>
          </template>
        </tbody>
      </table>
    </section>
    <div v-else class="empty">还没有订阅组 — 在下方「抓取新订阅」创建。</div>

    <!-- new subscription -->
    <section class="new">
      <details>
        <summary>＋ 抓取新订阅</summary>
        <div class="new-form">
          <div class="row">
            <input v-model="url" class="inp" placeholder="https://example.com/xxxx/sub 或 ...?clash=2" @keydown.enter="doFetch" />
            <button :disabled="fetchBusy || !url.trim()" @click="doFetch">{{ fetchBusy ? "抓取中…" : "抓取解析" }}</button>
          </div>
          <div class="grid">
            <label>User-Agent</label>
            <input v-model="ua" class="inp mono" placeholder="如 clash-verge/v2.5.2" />
            <label>额外请求头</label>
            <textarea v-model="extraHeaders" rows="2" class="inp mono" placeholder="Referer: https://example.com&#10;每行 Key: Value" />
          </div>
          <div v-if="preview" class="preview">
            <span>解析 {{ preview.nodes.length }} 节点 · 错误 {{ preview.errors.length }}</span>
            <input v-model="saveName" class="inp mono" placeholder="保存为订阅组名称" />
            <button :disabled="!saveName.trim()" @click="doSaveNew">保存为新订阅组</button>
          </div>
        </div>
      </details>
    </section>
  </div>
</template>

<style scoped>
.sub-page { display: flex; flex-direction: column; gap: 12px; max-width: 980px; }
.head { display: flex; align-items: baseline; gap: 12px; }
.head h1 { margin: 0 8px 0 0; font-size: 20px; }
.hint { opacity: 0.6; font-size: 12px; }
.spacer { flex: 1; }
.err { color: #ef4444; font-size: 12px; }
.ok { color: #22c55e; font-size: 12px; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; padding: 7px 8px; border-top: 1px solid var(--border); }
th { font-size: 11px; text-transform: uppercase; opacity: 0.7; }
.url { max-width: 380px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.mono { font-family: ui-monospace, monospace; font-size: 11px; }
.dim { opacity: 0.5; }
.ops { white-space: nowrap; text-align: right; }
button {
  border: 0; border-radius: 6px; background: var(--accent); color: #fff;
  padding: 5px 12px; font-size: 12px; cursor: pointer;
}
button:disabled { opacity: 0.5; cursor: default; }
.ghost { background: transparent; border: 1px solid var(--border); color: inherit; }
.danger { color: #ef4444; border-color: rgba(239, 68, 68, 0.4); }
.mini { padding: 2px 8px; font-size: 12px; margin-left: 4px; }
.editor-row td { background: rgba(0, 0, 0, 0.03); }
.editor { display: flex; flex-direction: column; gap: 8px; }
.grid { display: grid; grid-template-columns: 110px 1fr; gap: 6px 10px; align-items: center; }
.grid label { font-size: 12px; opacity: 0.85; }
.inp {
  border: 1px solid var(--border); border-radius: 6px; padding: 6px 8px;
  background: transparent; color: inherit; font-size: 12px; width: 100%;
  font-family: inherit;
}
.inp.mono { font-family: ui-monospace, monospace; font-size: 11px; }
.btns { display: flex; gap: 8px; }
.empty { padding: 18px; text-align: center; opacity: 0.55; font-size: 13px; border: 1px dashed var(--border); border-radius: 8px; }
.new summary { cursor: pointer; font-size: 14px; margin-bottom: 8px; }
.new-form { display: flex; flex-direction: column; gap: 8px; max-width: 760px; }
.row { display: flex; gap: 8px; }
.preview { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; font-size: 12px; }
</style>
