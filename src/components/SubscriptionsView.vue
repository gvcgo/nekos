<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  copyNodes,
  createGroup,
  deleteGroup,
  groupsList,
  nodesList,
  subscribe,
  subscriptionEdit,
  subscriptionRefresh,
  type Group,
  type Node,
  type SubUserInfo,
  type SubscribeResult,
} from "../api";

const emit = defineEmits<{ (e: "saved", groupId: number): void }>();

// ---- existing subscription list ----------------------------------------

const subs = ref<Group[]>([]);
const allGroups = ref<Group[]>([]);
const refreshing = ref<Record<number, boolean>>({});
const err = ref("");
const msg = ref("");

const subscriptions = computed(() =>
  subs.value.filter((g) => (g.sub_url ?? "").trim().length > 0),
);

async function loadSubs() {
  subs.value = await groupsList();
  allGroups.value = subs.value;
}

// ---- copy subscription nodes into a group ----

const copyOf = ref<Group | null>(null);
const copyNodesList = ref<Node[]>([]);
const copySel = ref<Record<string, boolean>>({});
const copyTarget = ref<number | null>(null);
const copyBusy = ref(false);

function targetGroups(): Group[] {
  if (!copyOf.value) return [];
  return allGroups.value.filter((g) => g.id !== copyOf.value!.id);
}

async function openCopy(g: Group) {
  copyOf.value = g;
  copyNodesList.value = await nodesList(g.id);
  const sel: Record<string, boolean> = {};
  for (const n of copyNodesList.value) sel[n.id] = true;
  copySel.value = sel;
  const t = targetGroups();
  copyTarget.value = t.length ? t[0].id : null;
  err.value = "";
}

function closeCopy() {
  copyOf.value = null;
  copyNodesList.value = [];
  copyTarget.value = null;
}

function toggleCopyAll(on: boolean) {
  const sel: Record<string, boolean> = {};
  for (const n of copyNodesList.value) sel[n.id] = on;
  copySel.value = sel;
}

async function doCopyToGroup() {
  if (!copyOf.value || copyTarget.value == null) {
    err.value = "请先创建目标分组（服务页「＋新建」）";
    return;
  }
  const ids = copyNodesList.value.filter((n) => copySel.value[n.id]).map((n) => n.id);
  if (!ids.length) {
    err.value = "没有勾选任何节点";
    return;
  }
  copyBusy.value = true;
  err.value = "";
  try {
    const out = await copyNodes(copyOf.value.id, copyTarget.value, ids);
    const targetName = allGroups.value.find((g) => g.id === copyTarget.value)?.name ?? "";
    msg.value = `已将 ${out.inserted} 个节点加入分组「${targetName}」${out.duplicated ? `，${out.duplicated} 个已存在跳过` : ""}`;
    closeCopy();
  } catch (e) {
    err.value = String(e);
  } finally {
    copyBusy.value = false;
  }
}

// ---- multi-subscription node picker ----

const multiOpen = ref(false);
const srcCheck = ref<Record<number, boolean>>({});
const srcCache = ref<Record<number, Node[]>>({});
const chosen = ref<Record<string, boolean>>({});
const query = ref("");
const multiTarget = ref<number | null>(null);
const multiBusy = ref(false);

const checkedSrcIds = computed(() =>
  subscriptions.value.filter((g) => srcCheck.value[g.id]).map((g) => g.id),
);

/** nodes of all checked sources, de-duplicated by id (keep first). */
const mergedNodes = computed<Node[]>(() => {
  const seen = new Set<string>();
  const out: Node[] = [];
  for (const id of checkedSrcIds.value) {
    for (const n of srcCache.value[id] ?? []) {
      if (!seen.has(n.id)) {
        seen.add(n.id);
        out.push(n);
      }
    }
  }
  return out;
});

const filteredMerged = computed(() => {
  const q = query.value.trim().toLowerCase();
  if (!q) return mergedNodes.value;
  return mergedNodes.value.filter(
    (n) =>
      n.remark.toLowerCase().includes(q) ||
      n.type.toLowerCase().includes(q),
  );
});

const chosenCount = computed(
  () => mergedNodes.value.filter((n) => chosen.value[n.id]).length,
);

function openMulti() {
  multiOpen.value = true;
  err.value = "";
  msg.value = "";
  query.value = "";
  const sc: Record<number, boolean> = {};
  for (const g of subscriptions.value) sc[g.id] = true;
  srcCheck.value = sc;
  srcCache.value = {};
  chosen.value = {};
  multiTarget.value = allGroups.value[0]?.id ?? null;
  void loadMultiSources();
}

function closeMulti() {
  multiOpen.value = false;
  srcCache.value = {};
}

async function loadMultiSources() {
  for (const id of checkedSrcIds.value) {
    if (srcCache.value[id]) continue;
    try {
      srcCache.value[id] = await nodesList(id);
      for (const n of srcCache.value[id]!) chosen.value[n.id] = true;
    } catch (e) {
      err.value = String(e);
    }
  }
}

async function onToggleSource(g: Group, on: boolean) {
  srcCheck.value[g.id] = on;
  if (on) await loadMultiSources();
}

function toggleAllVisible(on: boolean) {
  for (const n of filteredMerged.value) chosen.value[n.id] = on;
}

async function doMultiCopy() {
  if (multiTarget.value == null) {
    err.value = "没有目标分组（先在服务页新建）";
    return;
  }
  const target = multiTarget.value;
  let added = 0;
  let dup = 0;
  multiBusy.value = true;
  err.value = "";
  try {
    for (const srcId of checkedSrcIds.value) {
      const idsHere = (srcCache.value[srcId] ?? [])
        .filter((n) => chosen.value[n.id])
        .map((n) => n.id);
      if (!idsHere.length) continue;
      const out = await copyNodes(srcId, target, idsHere);
      added += out.inserted;
      dup += out.duplicated;
    }
    const targetName = allGroups.value.find((g) => g.id === target)?.name ?? "";
    msg.value = `已合并导入 ${added} 个节点到「${targetName}」${dup ? `（${dup} 个已存在跳过）` : ""}`;
    closeMulti();
  } catch (e) {
    err.value = String(e);
  } finally {
    multiBusy.value = false;
  }
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
  if (!confirm(`删除订阅「${g.name}」及其自带分组节点？\n已加入其它分组的节点副本会保留。`)) return;
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
      <button class="mini accent" @click="openMulti">多订阅选节点入组…</button>
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
                <button class="ghost mini" @click="openCopy(g)">加入分组…</button>
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
    <div class="empty" v-if="!subscriptions.length && !copyOf">还没有订阅组 — 在下方「抓取新订阅」创建。</div>

    <!-- copy subscription nodes into a group -->
    <div v-if="copyOf" class="dialog-mask" @click.self="closeCopy">
      <div class="dialog">
        <h3>将「{{ copyOf.name }}」的节点加入分组</h3>
        <div class="dialog-row">
          <label>目标分组</label>
          <select v-model="copyTarget" class="inp sel">
            <option v-for="g in targetGroups()" :key="g.id" :value="g.id">{{ g.name }}</option>
          </select>
        </div>
        <div class="dialog-actions">
          <span class="spacer"></span>
          <label class="check"><input type="checkbox" :checked="copyNodesList.every((n) => copySel[n.id])" @change="toggleCopyAll(($event.target as HTMLInputElement).checked)" /> 全选</label>
        </div>
        <div class="node-pick">
          <label v-for="n in copyNodesList" :key="n.id" class="pick">
            <input type="checkbox" v-model="copySel[n.id]" />
            <code>{{ n.type }}</code> {{ n.remark }}
          </label>
        </div>
        <div class="dialog-btns">
          <span v-if="err" class="err">{{ err }}</span>
          <span class="spacer"></span>
          <button class="ghost" @click="closeCopy">取消</button>
          <button :disabled="copyBusy || copyTarget == null" @click="doCopyToGroup">
            {{ copyBusy ? "加入中…" : `加入 ${copyNodesList.filter((n) => copySel[n.id]).length} 个节点` }}
          </button>
        </div>
      </div>
    </div>

    <!-- multi-subscription node picker -->
    <div v-if="multiOpen" class="dialog-mask" @click.self="closeMulti">
      <div class="dialog wide">
        <h3>从多个订阅选择节点导入分组</h3>
        <div class="multi-layout">
          <div class="src-col">
            <div class="src-title">订阅源</div>
            <label v-for="g in subscriptions" :key="g.id" class="pick">
              <input type="checkbox" :checked="srcCheck[g.id]" @change="onToggleSource(g, ($event.target as HTMLInputElement).checked)" />
              <span class="src-name">{{ g.name }}</span>
              <span v-if="srcCache[g.id]" class="dim">{{ srcCache[g.id]!.length }}</span>
            </label>
          </div>
          <div class="node-col">
            <div class="dialog-row">
              <input v-model="query" class="inp" placeholder="搜索备注 / 类型（如 HKG、trojan）" />
            </div>
            <div class="dialog-actions">
              <span>已选 {{ chosenCount }} / {{ mergedNodes.length }}</span>
              <span class="spacer"></span>
              <button class="ghost mini" @click="toggleAllVisible(true)">全选(当前筛选)</button>
              <button class="ghost mini" @click="toggleAllVisible(false)">清空筛选</button>
            </div>
            <div class="node-pick">
              <label v-for="n in filteredMerged.slice(0, 600)" :key="n.id" class="pick">
                <input type="checkbox" v-model="chosen[n.id]" />
                <code>{{ n.type }}</code> {{ n.remark }}
              </label>
              <div v-if="filteredMerged.length > 600" class="truncated">
                仅展示前 600 条（共 {{ filteredMerged.length }}），搜索可缩小范围
              </div>
              <div v-if="!filteredMerged.length" class="truncated">没有匹配节点</div>
            </div>
          </div>
        </div>
        <div class="dialog-row">
          <label>导入到分组</label>
          <select v-model="multiTarget" class="inp sel">
            <option v-for="g in allGroups" :key="g.id" :value="g.id">{{ g.name }}</option>
          </select>
          <span class="spacer"></span>
          <span v-if="err" class="err">{{ err }}</span>
        </div>
        <div class="dialog-btns">
          <span class="spacer"></span>
          <button class="ghost" @click="closeMulti">取消</button>
          <button :disabled="multiBusy || multiTarget == null || !chosenCount" @click="doMultiCopy">
            {{ multiBusy ? "导入中…" : `导入 ${chosenCount} 个节点` }}
          </button>
        </div>
      </div>
    </div>

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
.dialog-mask {
  position: fixed; inset: 0; background: rgba(0, 0, 0, 0.45);
  display: flex; align-items: center; justify-content: center; z-index: 50;
}
.dialog {
  background: var(--bg, #f5f6f8); color: var(--fg, #1f2329);
  border: 1px solid var(--border); border-radius: 12px; padding: 16px;
  width: min(620px, 92vw); max-height: 80vh; display: flex; flex-direction: column; gap: 10px;
}
.dialog h3 { margin: 0; font-size: 15px; }
.dialog.wide { width: min(860px, 94vw); }
.multi-layout { display: flex; gap: 12px; min-height: 320px; }
.src-col { width: 220px; flex: none; border: 1px solid var(--border); border-radius: 8px; padding: 8px; display: flex; flex-direction: column; gap: 4px; overflow: auto; }
.src-title { font-size: 11px; text-transform: uppercase; opacity: 0.6; margin-bottom: 2px; }
.src-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.node-col { flex: 1; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
.node-col .dialog-actions { font-size: 12px; }
button.accent { background: var(--accent); color: #fff; border: 0; }
button.mini { padding: 2px 8px; font-size: 12px; margin-left: 4px; }
.truncated { padding: 4px 2px; font-size: 11px; opacity: 0.6; }
.dialog-row { display: flex; align-items: center; gap: 10px; }
.dialog-row label { font-size: 12px; opacity: 0.85; width: 70px; flex: none; }
.sel { max-width: 260px; }
.dialog-actions, .dialog-btns { display: flex; align-items: center; gap: 10px; }
.dialog-actions .spacer, .dialog-btns .spacer { flex: 1; }
.check { display: inline-flex; align-items: center; gap: 5px; font-size: 12px; cursor: pointer; }
.node-pick {
  flex: 1; overflow: auto; border: 1px solid var(--border); border-radius: 8px; padding: 6px 8px;
  display: flex; flex-direction: column; gap: 2px; min-height: 120px;
}
.pick { display: flex; align-items: center; gap: 8px; font-size: 12px; cursor: pointer; }
.pick code { font-size: 11px; }

</style>
