<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  copyNodes,
  createGroup,
  deleteGroup,
  groupsList,
  nodesList,
  settingsGet,
  subscribe,
  subscriptionEdit,
  subscriptionRefresh,
  type Group,
  type Node,
  type SubUserInfo,
  type SubscribeResult,
} from "../api";
import { useDict, fmt, type Dict } from "../i18n";

const zhL = {
  title: "订阅",
  subCount: "{n} 个订阅组",
  autoOn: "自动更新开（{minutes} 分钟）",
  updating: "更新中…",
  refreshAll: "全部更新",
  multiPick: "多订阅选节点入组…",
  allUpdated: "全部订阅更新完成",
  name: "名称",
  colUrl: "地址",
  colTraffic: "流量",
  update: "更新",
  editTip: "编辑 URL/Header",
  copyTip: "将本订阅节点加入分组",
  refreshNow: "立即抓取更新",
  viewNodes: "查看节点",
  deleteSub: "删除订阅",
  dueShort: "已到期",
  dash: "—",
  secAgo: "{n}秒前",
  minAgo: "{n}分钟前",
  hourAgo: "{n}小时前",
  dayAgo: "{n}天前",
  countMin: "{n}分钟",
  countHour: "{n}小时",
  quotaUsed: "已用: {up}↑ / {down}↓",
  quotaTotal: "总量: {total}",
  quotaExpire: "到期: {date}",
  lastUpdatedAt: "上次更新: {at}",
  neverUpdated: "从未更新过",
  dueAuto: "已到期，后台将自动更新",
  nextAutoAt: "下次自动更新: {date}",
  delConfirm: "删除订阅「{name}」及其自带分组节点？\n已加入其它分组的节点副本会保留。",
  editSaved: "已保存订阅配置（点「更新」按新配置重新抓取）",
  subUrl: "订阅地址",
  userAgent: "User-Agent",
  extraHeaders: "额外请求头",
  editUrlPh: "https://example.com/sub 或 ...?clash=2",
  uaPh: "如 clash-verge/v2.5.2（留空则不发送）",
  extrasPh: "Referer: https://example.com\nAuthorization: Bearer xxx",
  saving: "保存中…",
  saveChanges: "保存修改",
  cancel: "取消",
  emptyState: "还没有订阅组 — 在下方「抓取新订阅」创建。",
  copyTitle: "将「{name}」的节点加入分组",
  targetGroup: "目标分组",
  selectAll: "全选",
  joining: "加入中…",
  joinCount: "加入 {n} 个节点",
  copyNeedTarget: "请先创建目标分组（分组页「＋新建」）",
  noNodePicked: "没有勾选任何节点",
  joinedMsg: "已将 {n} 个节点加入分组「{name}」",
  dupSkipComma: "，{m} 个已存在跳过",
  multiTitle: "从多个订阅选择节点导入分组",
  srcTitle: "订阅源",
  searchPh: "搜索备注 / 类型（如 HKG、trojan）",
  selCount: "已选 {n} / {m}",
  selectFiltered: "全选(当前筛选)",
  clearSel: "清空筛选",
  showTrunc: "仅展示前 600 条（共 {n}），搜索可缩小范围",
  noMatch: "没有匹配节点",
  importToGroup: "导入到分组",
  importing: "导入中…",
  importCount: "导入 {n} 个节点",
  multiNeedTarget: "没有目标分组（先在服务页新建）",
  multiImported: "已合并导入 {n} 个节点到「{name}」",
  dupSkipParen: "（{m} 个已存在跳过）",
  fetchNew: "＋ 抓取新订阅",
  subUrlPh: "https://example.com/xxxx/sub 或 ...?clash=2",
  fetching: "抓取中…",
  fetchParse: "抓取解析",
  uaExamplePh: "如 clash-verge/v2.5.2",
  headersPh: "Referer: https://example.com\n每行 Key: Value",
  parseSummary: "解析 {n} 节点 · 错误 {m}",
  newGroupNamePh: "保存为订阅组名称",
  saveNewGroup: "保存为新订阅组",
  subCreated: "已创建订阅「{name}」（{n} 节点）",
  subRefreshed: "「{name}」已更新：{n} 节点，{m} 错误",
  newSubDefault: "新订阅",
} as const;
type DictKeys = keyof typeof zhL;
const enL: Record<DictKeys, string> = {
  title: "Subscriptions",
  subCount: "{n} subscription groups",
  autoOn: "Auto-update on ({minutes} min)",
  updating: "Updating…",
  refreshAll: "Refresh all",
  multiPick: "Pick nodes from multiple subscriptions…",
  allUpdated: "All subscriptions updated",
  name: "Name",
  colUrl: "URL",
  colTraffic: "Traffic",
  update: "Update",
  editTip: "Edit URL/Header",
  copyTip: "Add this subscription's nodes to a group",
  refreshNow: "Fetch update now",
  viewNodes: "View nodes",
  deleteSub: "Delete subscription",
  dueShort: "Due",
  dash: "—",
  secAgo: "{n}s ago",
  minAgo: "{n}m ago",
  hourAgo: "{n}h ago",
  dayAgo: "{n}d ago",
  countMin: "{n} min",
  countHour: "{n} hr",
  quotaUsed: "Used: {up}↑ / {down}↓",
  quotaTotal: "Total: {total}",
  quotaExpire: "Expires: {date}",
  lastUpdatedAt: "Last updated: {at}",
  neverUpdated: "Never updated",
  dueAuto: "Due — will auto-update in the background",
  nextAutoAt: "Next auto-update: {date}",
  delConfirm: "Delete subscription “{name}” and its own group nodes?\nCopies added to other groups will be kept.",
  editSaved: "Subscription config saved — click “Update” to re-fetch with the new config",
  subUrl: "Subscription URL",
  userAgent: "User-Agent",
  extraHeaders: "Extra headers",
  editUrlPh: "https://example.com/sub or ...?clash=2",
  uaPh: "e.g. clash-verge/v2.5.2 (leave blank to not send)",
  extrasPh: "Referer: https://example.com\nAuthorization: Bearer xxx",
  saving: "Saving…",
  saveChanges: "Save changes",
  cancel: "Cancel",
  emptyState: "No subscription groups yet — create one with “Fetch new subscription” below.",
  copyTitle: "Add nodes of “{name}” to a group",
  targetGroup: "Target group",
  selectAll: "Select all",
  joining: "Adding…",
  joinCount: "Add {n} nodes",
  copyNeedTarget: "Create a target group first (Servers page “+ New”)",
  noNodePicked: "No nodes selected",
  joinedMsg: "Added {n} nodes to group “{name}”",
  dupSkipComma: " ({m} already exist — skipped)",
  multiTitle: "Pick nodes from multiple subscriptions and import them into a group",
  srcTitle: "Sources",
  searchPh: "Search remark / type (e.g. HKG, trojan)",
  selCount: "Selected {n} / {m}",
  selectFiltered: "Select all (filtered)",
  clearSel: "Clear selection",
  showTrunc: "Only the first 600 are shown (of {n}) — search to narrow the range",
  noMatch: "No matching nodes",
  importToGroup: "Import to group",
  importing: "Importing…",
  importCount: "Import {n} nodes",
  multiNeedTarget: "No target group (create one on the Servers page first)",
  multiImported: "Merged-imported {n} nodes into “{name}”",
  dupSkipParen: " ({m} already exist — skipped)",
  fetchNew: "＋ Fetch new subscription",
  subUrlPh: "https://example.com/xxxx/sub or ...?clash=2",
  fetching: "Fetching…",
  fetchParse: "Fetch & parse",
  uaExamplePh: "e.g. clash-verge/v2.5.2",
  headersPh: "Referer: https://example.com\nOne “Key: Value” pair per line",
  parseSummary: "Parsed {n} nodes · {m} errors",
  newGroupNamePh: "Name for the new subscription group",
  saveNewGroup: "Save as new subscription group",
  subCreated: "Created subscription “{name}” ({n} nodes)",
  subRefreshed: "“{name}” updated: {n} nodes, {m} errors",
  newSubDefault: "New subscription",
};
const dict = useDict({ zh: zhL as Dict, en: enL });
function tt(k: DictKeys, p?: Record<string, string | number>): string {
  return fmt(dict.value[k] as string, p);
}

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

// ---- auto-update visibility --------------------------------------------

const autoEnabled = ref(false);
const autoMinutes = ref(360);
const refreshingAll = ref(false);

onMounted(async () => {
  await loadSubs();
  try {
    const s = await settingsGet();
    autoEnabled.value = s.auto_update_subscriptions ?? false;
    autoMinutes.value = s.auto_update_minutes ?? 360;
  } catch {
    /* non-fatal */
  }
});

function autoInfo(g: Group): { due: boolean; next: number } | null {
  if (!autoEnabled.value || g.last_update_epoch == null) return null;
  const next = (g.last_update_epoch + autoMinutes.value * 60) * 1000;
  return { due: Date.now() >= next, next };
}

async function refreshAll() {
  refreshingAll.value = true;
  err.value = "";
  msg.value = "";
  try {
    for (const g of subscriptions.value) {
      try {
        await refreshSub(g);
      } catch {
        // refreshSub shows its own errors; keep going for the rest
      }
    }
    msg.value = tt("allUpdated");
  } finally {
    refreshingAll.value = false;
  }
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
    err.value = tt("copyNeedTarget");
    return;
  }
  const ids = copyNodesList.value.filter((n) => copySel.value[n.id]).map((n) => n.id);
  if (!ids.length) {
    err.value = tt("noNodePicked");
    return;
  }
  copyBusy.value = true;
  err.value = "";
  try {
    const out = await copyNodes(copyOf.value.id, copyTarget.value, ids);
    const targetName = allGroups.value.find((g) => g.id === copyTarget.value)?.name ?? "";
    msg.value = tt("joinedMsg", { n: out.inserted, name: targetName }) + (out.duplicated ? tt("dupSkipComma", { m: out.duplicated }) : "");
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
    err.value = tt("multiNeedTarget");
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
    msg.value = tt("multiImported", { n: added, name: targetName }) + (dup ? tt("dupSkipParen", { m: dup }) : "");
    closeMulti();
  } catch (e) {
    err.value = String(e);
  } finally {
    multiBusy.value = false;
  }
}

function fmtRelTime(epoch?: number): string {
  if (!epoch) return tt("dash");
  const s = Math.max(0, Math.floor(Date.now() / 1000 - epoch));
  if (s < 60) return tt("secAgo", { n: s });
  const m = Math.floor(s / 60);
  if (m < 60) return tt("minAgo", { n: m });
  const h = Math.floor(m / 60);
  if (h < 24) return tt("hourAgo", { n: h });
  return tt("dayAgo", { n: Math.floor(h / 24) });
}

function fmtCountdown(nextMs: number): string {
  const min = Math.max(1, Math.ceil((nextMs - Date.now()) / 60000));
  return min < 60 ? tt("countMin", { n: min }) : tt("countHour", { n: Math.round(min / 60) });
}

function quotaTooltip(g: Group): string | undefined {
  const u = userInfo(g);
  if (!u) return undefined;
  const lines = [
    tt("quotaUsed", { up: fmtBytes(u.upload), down: fmtBytes(u.download) }),
    tt("quotaTotal", { total: fmtBytes(u.total) }),
  ];
  if (u.expire) lines.push(tt("quotaExpire", { date: new Date(u.expire * 1000).toLocaleString() }));
  return lines.join("\n");
}

function updatedTooltip(g: Group): string | undefined {
  const parts: string[] = [g.updated_at ? tt("lastUpdatedAt", { at: g.updated_at }) : tt("neverUpdated")];
  const a = autoInfo(g);
  if (a) {
    parts.push(a.due ? tt("dueAuto") : tt("nextAutoAt", { date: new Date(a.next).toLocaleString() }));
  }
  return parts.join("\n");
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
  if (!confirm(tt("delConfirm", { name: g.name }))) return;
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
    msg.value = tt("subRefreshed", { name: g.name, n: out.nodes.length, m: out.errors.length });
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
    msg.value = tt("editSaved");
    await loadSubs();
  } catch (e) {
    err.value = String(e);
  } finally {
    savingEdit.value = false;
  }
}

// ---- new subscription ---------------------------------------------------

const url = ref("");
const ua = ref("");
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
    return tt("newSubDefault");
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
      msg.value = tt("subCreated", { name: saveName.value.trim(), n: out.nodes.length });
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

</script>

<template>
  <div class="sub-page">
    <div class="head">
      <h1>{{ tt("title") }}</h1>
      <span class="hint">{{ tt("subCount", { n: subscriptions.length }) }}</span>
      <button class="mini accent" @click="refreshAll" :disabled="refreshingAll">{{ refreshingAll ? tt("updating") : tt("refreshAll") }}</button>
      <button class="mini accent" @click="openMulti">{{ tt("multiPick") }}</button>
      <span v-if="autoEnabled" class="hint">{{ tt("autoOn", { minutes: autoMinutes }) }}</span>
      <span class="spacer"></span>
      <span v-if="msg" class="ok">{{ msg }}</span>
      <span v-if="err" class="err">{{ err }}</span>
    </div>

    <!-- managed subscription list -->
    <section v-if="subscriptions.length" class="list">
      <table>
        <thead>
          <tr><th class="c-name">{{ tt("name") }}</th><th class="c-url">{{ tt("colUrl") }}</th><th class="c-meta">{{ tt("colTraffic") }}</th><th class="c-meta">{{ tt("update") }}</th><th class="c-ops"></th></tr>
        </thead>
        <tbody>
          <template v-for="g in subscriptions" :key="g.id">
            <tr>
              <td class="c-name ell" :title="g.name">{{ g.name }}</td>
              <td class="c-url ell mono" :title="g.sub_url">{{ g.sub_url }}</td>
              <td class="c-meta ell mono" :title="quotaTooltip(g)">
                <template v-if="userInfo(g)">
                  {{ fmtBytes((userInfo(g)!.upload || 0) + (userInfo(g)!.download || 0)) }}/{{ fmtBytes(userInfo(g)!.total || 0) }}
                </template>
                <span v-else class="dim">{{ tt("dash") }}</span>
              </td>
              <td class="c-meta ell" :title="updatedTooltip(g)">
                <span :class="{ 'auto-due': autoInfo(g)?.due }">
                  {{ fmtRelTime(g.last_update_epoch) }}
                  <span v-if="autoInfo(g) && !autoInfo(g)!.due" class="dim">({{ fmtCountdown(autoInfo(g)!.next) }})</span>
                  <span v-else-if="autoInfo(g)?.due">{{ tt("dueShort") }}</span>
                </span>
              </td>
              <td class="ops">
                <button class="ghost mini icon-btn" :title="tt('editTip')" @click="startEdit(g)">✎</button>
                <button class="ghost mini icon-btn" :title="tt('copyTip')" @click="openCopy(g)">＋</button>
                <button class="mini" :disabled="refreshing[g.id]" :title="refreshing[g.id] ? tt('updating') : tt('refreshNow')" @click="refreshSub(g)">
                  {{ refreshing[g.id] ? "…" : tt("update") }}
                </button>
                <button class="ghost mini icon-btn" :title="tt('viewNodes')" @click="emit('saved', g.id)">▸</button>
                <button class="ghost mini icon-btn danger" :title="tt('deleteSub')" @click="delSub(g)">✕</button>
              </td>
            </tr>

            <!-- inline editor -->
            <tr v-if="editingId === g.id" class="editor-row">
              <td colspan="5">
                <div class="editor">
                  <div class="grid">
                    <label>{{ tt("name") }}</label>
                    <input v-model="editName" class="inp" />
                    <label>{{ tt("subUrl") }}</label>
                    <input v-model="editUrl" class="inp mono" :placeholder="tt('editUrlPh')" />
                    <label>{{ tt("userAgent") }}</label>
                    <input v-model="editUA" class="inp mono" :placeholder="tt('uaPh')" />
                    <label>{{ tt("extraHeaders") }}</label>
                    <textarea v-model="editExtrasText" rows="2" class="inp mono" :placeholder="tt('extrasPh')" />
                  </div>
                  <div class="btns">
                    <button :disabled="savingEdit" @click="saveEdit">{{ savingEdit ? tt("saving") : tt("saveChanges") }}</button>
                    <button class="ghost" @click="cancelEdit">{{ tt("cancel") }}</button>
                  </div>
                </div>
              </td>
            </tr>
          </template>
        </tbody>
      </table>
    </section>
    <div class="empty" v-if="!subscriptions.length && !copyOf">{{ tt("emptyState") }}</div>

    <!-- copy subscription nodes into a group -->
    <div v-if="copyOf" class="dialog-mask" @click.self="closeCopy">
      <div class="dialog">
        <h3>{{ tt("copyTitle", { name: copyOf.name }) }}</h3>
        <div class="dialog-row">
          <label>{{ tt("targetGroup") }}</label>
          <select v-model="copyTarget" class="inp sel">
            <option v-for="g in targetGroups()" :key="g.id" :value="g.id">{{ g.name }}</option>
          </select>
        </div>
        <div class="dialog-actions">
          <span class="spacer"></span>
          <label class="check"><input type="checkbox" :checked="copyNodesList.every((n) => copySel[n.id])" @change="toggleCopyAll(($event.target as HTMLInputElement).checked)" /> {{ tt("selectAll") }}</label>
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
          <button class="ghost" @click="closeCopy">{{ tt("cancel") }}</button>
          <button :disabled="copyBusy || copyTarget == null" @click="doCopyToGroup">
            {{ copyBusy ? tt("joining") : tt("joinCount", { n: copyNodesList.filter((n) => copySel[n.id]).length }) }}
          </button>
        </div>
      </div>
    </div>

    <!-- multi-subscription node picker -->
    <div v-if="multiOpen" class="dialog-mask" @click.self="closeMulti">
      <div class="dialog wide">
        <h3>{{ tt("multiTitle") }}</h3>
        <div class="multi-layout">
          <div class="src-col">
            <div class="src-title">{{ tt("srcTitle") }}</div>
            <label v-for="g in subscriptions" :key="g.id" class="pick">
              <input type="checkbox" :checked="srcCheck[g.id]" @change="onToggleSource(g, ($event.target as HTMLInputElement).checked)" />
              <span class="src-name">{{ g.name }}</span>
              <span v-if="srcCache[g.id]" class="dim">{{ srcCache[g.id]!.length }}</span>
            </label>
          </div>
          <div class="node-col">
            <div class="dialog-row">
              <input v-model="query" class="inp" :placeholder="tt('searchPh')" />
            </div>
            <div class="dialog-actions">
              <span>{{ tt("selCount", { n: chosenCount, m: mergedNodes.length }) }}</span>
              <span class="spacer"></span>
              <button class="ghost mini" @click="toggleAllVisible(true)">{{ tt("selectFiltered") }}</button>
              <button class="ghost mini" @click="toggleAllVisible(false)">{{ tt("clearSel") }}</button>
            </div>
            <div class="node-pick">
              <label v-for="n in filteredMerged.slice(0, 600)" :key="n.id" class="pick">
                <input type="checkbox" v-model="chosen[n.id]" />
                <code>{{ n.type }}</code> {{ n.remark }}
              </label>
              <div v-if="filteredMerged.length > 600" class="truncated">
                {{ tt("showTrunc", { n: filteredMerged.length }) }}
              </div>
              <div v-if="!filteredMerged.length" class="truncated">{{ tt("noMatch") }}</div>
            </div>
          </div>
        </div>
        <div class="dialog-row">
          <label>{{ tt("importToGroup") }}</label>
          <select v-model="multiTarget" class="inp sel">
            <option v-for="g in allGroups" :key="g.id" :value="g.id">{{ g.name }}</option>
          </select>
          <span class="spacer"></span>
          <span v-if="err" class="err">{{ err }}</span>
        </div>
        <div class="dialog-btns">
          <span class="spacer"></span>
          <button class="ghost" @click="closeMulti">{{ tt("cancel") }}</button>
          <button :disabled="multiBusy || multiTarget == null || !chosenCount" @click="doMultiCopy">
            {{ multiBusy ? tt("importing") : tt("importCount", { n: chosenCount }) }}
          </button>
        </div>
      </div>
    </div>

    <!-- new subscription -->
    <section class="new">
      <details>
        <summary>{{ tt("fetchNew") }}</summary>
        <div class="new-form">
          <div class="row">
            <input v-model="url" class="inp" :placeholder="tt('subUrlPh')" @keydown.enter="doFetch" />
            <button :disabled="fetchBusy || !url.trim()" @click="doFetch">{{ fetchBusy ? tt("fetching") : tt("fetchParse") }}</button>
          </div>
          <div class="grid">
            <label>{{ tt("userAgent") }}</label>
            <input v-model="ua" class="inp mono" :placeholder="tt('uaExamplePh')" />
            <label>{{ tt("extraHeaders") }}</label>
            <textarea v-model="extraHeaders" rows="2" class="inp mono" :placeholder="tt('headersPh')" />
          </div>
          <div v-if="preview" class="preview">
            <span>{{ tt("parseSummary", { n: preview.nodes.length, m: preview.errors.length }) }}</span>
            <input v-model="saveName" class="inp mono" :placeholder="tt('newGroupNamePh')" />
            <button :disabled="!saveName.trim()" @click="doSaveNew">{{ tt("saveNewGroup") }}</button>
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
th, td { text-align: start; padding: 7px 8px; border-top: 1px solid var(--border); }
th { font-size: 11px; text-transform: uppercase; opacity: 0.7; }
.url { max-width: 380px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.mono { font-family: ui-monospace, monospace; font-size: 11px; }
.dim { opacity: 0.5; }
.auto-due { color: #f59e0b; }
.ell { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.list table { table-layout: fixed; }
th.c-name { width: 26%; }
th.c-url { width: 22%; }
th.c-meta { width: 12%; }
th.c-ops { width: 28%; }
.c-ops button { margin-left: 4px; }
.icon-btn { font-size: 12px; line-height: 1; padding: 2px 5px; }
tr:hover td { background: rgba(59, 130, 246, 0.04); }
.ops { white-space: nowrap; text-align: end; }
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
/* URL input flexes instead of claiming 100% (which squeezed the button into
   wrapped text); the button keeps its natural width on one line. */
.row .inp { width: auto; flex: 1 1 380px; min-width: 0; }
.row button { flex: 0 0 auto; white-space: nowrap; }
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
