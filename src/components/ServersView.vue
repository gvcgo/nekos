<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  coreStatus,
  coreStop,
  coreStart,
  copyNodes,
  createGroup,
  createStrategyGroup,
  deleteGroup,
  deleteNode,
  groupsList,
  importToGroup,
  latencyList,
  measureBatch,
  measureNode,
  nodesList,
  nodeEncode,
  nodeQr,
  proxySet,
  renameGroup,
  setNodeCurrent,
  settingsGet,
  settingsSet,
  type CoreStatusView,
  type Group,
  type MeasureView,
  type Node,
  type Settings,
} from "../api";

const groups = ref<Group[]>([]);
const settings = ref<Settings | null>(null);
const nodes = ref<Node[]>([]);
const status = ref<CoreStatusView>({ running: false, proxy_enabled: false });

const importText = ref("");
const importBusy = ref(false);
const importMsg = ref("");
const startBusy = ref(false);
const err = ref("");
const delayMap = ref<Record<string, MeasureView>>({});
const testingAll = ref(false);
const sortByDelay = ref(true);

/** v6 heuristic mirror of the Rust filter (server without port). */
function nodeIsV6(n: Node): boolean {
  try {
    const out = JSON.parse(n.out) as { server?: string };
    const raw = out.server ?? "";
    let host = raw.trim();
    if (host.startsWith("[")) {
      const end = host.indexOf("]");
      if (end > 0) host = host.slice(1, end);
    }
    const pct = host.indexOf("%");
    if (pct > 0) host = host.slice(0, pct);
    if (!host.includes(":")) return false;
    return !/^\d{1,3}(\.\d{1,3}){3}$/.test(host);
  } catch {
    return false;
  }
}

const visibleNodes = computed(() => {
  if (!(settings.value?.filter_ipv6 ?? false)) return nodes.value;
  return nodes.value.filter((n) => !nodeIsV6(n));
});

/** latency ordering: measured asc; failed after measured; untested last. */
const sortedNodes = computed(() => {
  const list = visibleNodes.value;
  if (!sortByDelay.value) return list;
  const rank = (n: Node): [number, number] => {
    const m = delayMap.value[n.id];
    if (m && m.delay_ms != null) return [0, m.delay_ms];
    if (m && m.error) return [1, 0];
    return [2, 0];
  };
  return [...list].sort((a, b) => {
    const [ra, va] = rank(a);
    const [rb, vb] = rank(b);
    return ra !== rb ? ra - rb : va - vb;
  });
});

async function toggleSort(on: boolean) {
  settings.value = await settingsSet({ sort_by_delay: on });
  sortByDelay.value = on;
}

function currentGroupId(): number {
  return settings.value?.current_group_id ?? 1;
}

function selectedNodeId(): string | undefined {
  if (!settings.value) return undefined;
  return settings.value.selected_by_group[currentGroupId()];
}

async function refreshStatus() {
  status.value = await coreStatus();
}

async function loadAll() {
  const [gs, st, cs] = await Promise.all([groupsList(), settingsGet(), coreStatus()]);
  groups.value = gs;
  settings.value = st;
  status.value = cs;
  sortByDelay.value = st.sort_by_delay ?? true;
  await reloadNodes();
}

async function reloadNodes() {
  const id = currentGroupId();
  nodes.value = await nodesList(id);
  // restore persisted latency results
  try {
    const rows = await latencyList(id);
    const dm: Record<string, MeasureView> = {};
    for (const r of rows) {
      dm[r.node_id] = { delay_ms: r.delay_ms ?? null, error: r.error ?? null };
    }
    delayMap.value = dm;
  } catch {
    delayMap.value = {};
  }
  const sel = selectedNodeId();
  if (sel && !nodes.value.find((n) => n.id === sel)) {
    // stale selection (node deleted): fall back to first
    await setNodeCurrent(id, nodes.value[0]?.id ?? "");
    await loadSettings();
  }
}

async function loadSettings() {
  settings.value = await settingsGet();
}

async function switchGroup(id: number) {
  settings.value = await settingsSet({ current_group_id: id });
  nodes.value = await nodesList(id);
}

async function doImport() {
  importBusy.value = true;
  importMsg.value = "";
  err.value = "";
  try {
    const res = await importToGroup(currentGroupId(), importText.value);
    importMsg.value = `导入 ${res.nodes.length} 个节点，错误 ${res.errors.length}`;
    if (res.errors.length) {
      err.value = res.errors.slice(0, 5).map((e) => e.reason).join("；");
    }
    importText.value = "";
    await reloadNodes();
  } catch (e) {
    err.value = String(e);
  } finally {
    importBusy.value = false;
  }
}

async function removeNode(nodeId: string) {
  await deleteNode(currentGroupId(), nodeId);
  await reloadNodes();
}

function currentGroup(): Group {
  return groups.value.find((g) => g.id === currentGroupId()) ?? groups.value[0];
}

const isVirtualCurrent = computed(() => (currentGroup()?.kind ?? "normal") !== "normal");

function kindTag(g: Group): string {
  if (g.kind === "strategy") return "⚡"; // auto urltest
  if (g.kind === "strategy-manual") return "◈";
  return "";
}

// ---- create strategy group ----

const stratOpen = ref(false);
const stratName = ref("");
const stratAuto = ref(true);
const stratMembers = ref<Record<number, boolean>>({});
const stratBusy = ref(false);

const normalGroups = computed(() =>
  groups.value.filter((g) => (g.kind ?? "normal") === "normal"),
);

function openStrategyDialog() {
  stratName.value = "";
  stratAuto.value = true;
  const m: Record<number, boolean> = {};
  for (const g of normalGroups.value) m[g.id] = true;
  stratMembers.value = m;
  stratOpen.value = true;
}

async function createStrategy() {
  stratBusy.value = true;
  err.value = "";
  try {
    const ids = normalGroups.value.filter((g) => stratMembers.value[g.id]).map((g) => g.id);
    if (!stratName.value.trim() || !ids.length) {
      err.value = "请填写名称并选择至少一个成员分组";
      return;
    }
    await createStrategyGroup(currentGroupId(), stratName.value.trim(), stratAuto.value, ids);
    stratOpen.value = false;
    await reloadNodes();
  } catch (e) {
    err.value = String(e);
  } finally {
    stratBusy.value = false;
  }
}

function stratIdFrom(n: Node): number {
  return Number(n.id.replace("strat:", "")) || 0;
}

async function deleteStrategyNode(n: Node) {
  const sid = stratIdFrom(n);
  if (!sid || !confirm(`删除策略组「${n.remark}」？`)) return;
  await deleteGroup(sid);
  await reloadNodes();
}

// join (mount) a strategy node into another group
const joinOf = ref<Node | null>(null);
const joinTarget = ref<number | null>(null);

function openJoin(n: Node) {
  joinOf.value = n;
  const others = groups.value.filter(
    (g) => (g.kind ?? "normal") === "normal" && g.id !== currentGroupId(),
  );
  joinTarget.value = others[0]?.id ?? null;
}

async function doJoin() {
  const n = joinOf.value;
  if (!n || joinTarget.value == null) {
    err.value = "没有可加入的分组";
    return;
  }
  try {
    const out = await copyNodes(currentGroupId(), joinTarget.value, [n.id]);
    const targetName = groups.value.find((g) => g.id === joinTarget.value)?.name ?? "";
    shareMsg.value = `已将「${n.remark}」加入分组「${targetName}」${out.duplicated ? "（已存在）" : ""}`;
    joinOf.value = null;
  } catch (e) {
    err.value = String(e);
  }
}

async function removeGroup() {
  const gid = currentGroupId();
  if (gid === 1) return;
  if (!confirm(`删除分组「${currentGroup().name}」及其全部节点？`)) return;
  await deleteGroup(gid);
  await loadAll();
}

async function newGroup() {
  const name = prompt("新分组名称", "");
  if (!name?.trim()) return;
  const g = await createGroup(name.trim());
  await loadAll();
  if (g.id) await switchGroup(g.id);
}

async function renameCurrentGroup() {
  const gid = currentGroupId();
  if (gid === 1) return;
  const name = prompt("重命名分组", currentGroup().name);
  if (!name?.trim()) return;
  await renameGroup(gid, name.trim());
  await loadAll();
}

const switchMsg = ref("");
const shareMsg = ref("");
const qrModal = ref<{ remark: string; link: string; dataUrl: string } | null>(null);
const qrBusy = ref(false);

async function copyText(text: string): Promise<boolean> {
  try {
    await writeText(text); // native (Rust), no DOM side effects
    return true;
  } catch {
    // last-resort fallback: fully offscreen, restore scroll, no focus ring
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.cssText = "position:fixed;top:0;left:-9999px;opacity:0";
    document.body.appendChild(ta);
    const sc = { x: window.scrollX, y: window.scrollY };
    let ok = false;
    try {
      ta.focus({ preventScroll: true });
      ta.select();
      ok = document.execCommand("copy");
    } finally {
      document.body.removeChild(ta);
      window.scrollTo(sc.x, sc.y);
    }
    return ok;
  }
}

async function copyQrLink() {
  if (!qrModal.value) return;
  const ok = await copyText(qrModal.value.link);
  shareMsg.value = ok ? "链接已复制到剪贴板" : "复制失败";
}

async function copyNodeLink(groupId: number, nodeId: string, remark: string) {
  shareMsg.value = "";
  err.value = "";
  try {
    const link = await nodeEncode(groupId, nodeId);
    const ok = await copyText(link);
    shareMsg.value = ok ? `已复制「${remark}」链接` : "复制失败（请使用 QR 弹窗内手动复制）";
  } catch (e) {
    err.value = String(e);
  }
}

async function showNodeQr(groupId: number, nodeId: string, remark: string) {
  qrBusy.value = true;
  err.value = "";
  qrModal.value = null;
  try {
    const link = await nodeEncode(groupId, nodeId);
    const dataUrl = await nodeQr(groupId, nodeId);
    qrModal.value = { remark, link, dataUrl };
  } catch (e) {
    err.value = String(e);
  } finally {
    qrBusy.value = false;
  }
}

async function toggleStart() {
  startBusy.value = true;
  err.value = "";
  switchMsg.value = "";
  try {
    if (status.value.running) {
      status.value = await coreStop();
    } else {
      const run = await coreStart();
      status.value = run.status;
    }
  } catch (e) {
    err.value = String(e);
  } finally {
    startBusy.value = false;
    await refreshStatus();
  }
}

async function pickNode(nodeId: string) {
  await setNodeCurrent(currentGroupId(), nodeId);
  await loadSettings();
  if (status.value.running) {
    // live switch: rebuild the core with the new selection
    startBusy.value = true;
    switchMsg.value = "";
    try {
      const run = await coreStart();
      status.value = run.status;
      switchMsg.value = `已切换代理到「${nodes.value.find((n) => n.id === nodeId)?.remark ?? nodeId}」`;
    } catch (e) {
      err.value = String(e);
    } finally {
      startBusy.value = false;
      await refreshStatus();
    }
  }
}

async function testNode(nodeId: string) {
  delayMap.value[nodeId] = { delay_ms: null, error: null };
  delayMap.value[nodeId] = await measureNode(currentGroupId(), nodeId);
}

async function testAll() {
  testingAll.value = true;
  err.value = "";
  try {
    // one core instance probes every node concurrently (v2rayN semantics)
    const rows = await measureBatch(currentGroupId());
    const dm: Record<string, MeasureView> = { ...delayMap.value };
    for (const r of rows) {
      dm[r.node_id] = { delay_ms: r.delay_ms ?? null, error: r.error ?? null };
    }
    delayMap.value = dm;
  } catch (e) {
    err.value = String(e);
  } finally {
    testingAll.value = false;
  }
}

async function toggleProxy(on: boolean) {
  err.value = "";
  switchMsg.value = "";
  try {
    status.value = await proxySet(on);
    if (on) switchMsg.value = "系统代理已开启（关闭「停止」或再点开关即可还原）";
  } catch (e) {
    err.value = String(e);
    await refreshStatus(); // revert checkbox to real state
  }
}

async function changeMode(mode: string) {
  settings.value = await settingsSet({ mode });
}

function delayText(n: Node): string {
  const m = delayMap.value[n.id];
  if (!m) return "—";
  if (m.delay_ms != null) return `${m.delay_ms} ms`;
  return "失败";
}

onMounted(loadAll);
</script>

<template>
  <div class="servers-page">
    <div class="toolbar">
      <h1>服务</h1>

      <select class="group-select" :value="currentGroupId()" @change="switchGroup(Number(($event.target as HTMLSelectElement).value))">
        <option v-for="g in groups.filter((x) => (x.kind ?? 'normal') === 'normal')" :key="g.id" :value="g.id">{{ g.name }}</option>
      </select>
      <span class="group-ops" title="管理分组">
        <button class="ghost mini" @click="newGroup">＋ 新建</button>
        <button class="ghost mini" title="按成员分组延迟自动选最快的组（v2rayN 策略组）" @click="openStrategyDialog">＋策略组</button>
        <button v-if="currentGroupId() !== 1" class="ghost mini" @click="renameCurrentGroup">✎ 改名</button>
        <button v-if="currentGroupId() !== 1" class="ghost mini danger" title="删除分组（含节点）" @click="removeGroup">✕ 删除</button>
      </span>

      <span class="spacer"></span>

      <label class="chip" :class="status.running ? 'on' : 'off'">
        <span class="dot"></span>
        {{ status.running ? `运行中 · 系统代理${status.proxy_enabled ? "开" : "关"}` : "已停止" }}
      </label>
      <select class="group-select" :value="settings?.mode ?? 'global'" title="分流模式" @change="changeMode(($event.target as HTMLSelectElement).value)">
        <option value="global">全局代理</option>
        <option value="rule">规则(绕过大陆)</option>
        <option value="direct">直连</option>
      </select>
      <label class="proxy-toggle" title="内核运行中可随时开关系统代理">
        <input type="checkbox" :checked="status.proxy_enabled" :disabled="!status.running" @change="toggleProxy(($event.target as HTMLInputElement).checked)" />
        系统代理
      </label>
      <button :disabled="startBusy || !visibleNodes.length" @click="toggleStart">
        {{ status.running ? "停止" : startBusy ? "启动中…" : "启动" }}
      </button>
    </div>

    <div class="msgbar">
      <span v-if="switchMsg" class="ok">{{ switchMsg }}</span>
      <span v-if="shareMsg" class="ok">{{ shareMsg }}</span>
      <span v-if="err" class="err">{{ err }}</span>
    </div>

    <section v-if="!isVirtualCurrent" class="import-card">
      <div class="row">
        <textarea v-model="importText" rows="2" placeholder="粘贴分享链接 / 订阅内容（base64、Clash、JSON），导入到当前分组" />
        <button :disabled="importBusy || !importText.trim()" @click="doImport">{{ importBusy ? "导入中…" : "导入到当前分组" }}</button>
      </div>
      <span v-if="importMsg" class="ok">{{ importMsg }}</span>
    </section>

    <section class="nodes">
      <div class="nodes-head">
        <strong>节点（{{ visibleNodes.length }}）</strong>
        <span v-if="settings?.filter_ipv6 && visibleNodes.length < nodes.length" class="dim">
          已隐藏 {{ nodes.length - visibleNodes.length }} 个 IPv6
        </span>
        <span class="spacer"></span>
        <label class="sort-toggle" title="按延迟升序排列（失败与未测在后）">
          <input type="checkbox" :checked="sortByDelay" @change="toggleSort(($event.target as HTMLInputElement).checked)" />
          按延迟排序
        </label>
        <button class="ghost" :disabled="testingAll || !visibleNodes.length" @click="testAll">
          {{ testingAll ? "测速中…" : "全部测速" }}
        </button>
      </div>
      <table>
        <thead>
          <tr><th></th><th>类型</th><th>备注</th><th>延迟</th><th>操作</th></tr>
        </thead>
        <tbody>
          <tr v-for="n in sortedNodes" :key="n.id" :class="{ current: n.id === selectedNodeId() }">
            <td class="sel" @click="pickNode(n.id)">{{ n.id === selectedNodeId() ? "●" : "○" }}</td>
            <td><code>{{ n.type }}</code></td>
            <td class="remark" @click="pickNode(n.id)">{{ n.remark }}</td>
            <td :class="delayMap[n.id]?.error ? 'bad' : ''">{{ delayText(n) }}</td>
            <td class="ops">
              <button class="ghost" :disabled="!!delayMap[n.id] && delayMap[n.id]!.delay_ms == null && !delayMap[n.id]!.error" @click="testNode(n.id)">测速</button>
              <template v-if="n.type !== 'strategy'">
                <button class="ghost" :disabled="qrBusy" title="复制分享链接" @click="copyNodeLink(n.group_id, n.id, n.remark)">复制</button>
                <button class="ghost" title="二维码分享" @click="showNodeQr(n.group_id, n.id, n.remark)">QR</button>
                <button v-if="!isVirtualCurrent" class="ghost danger" @click="removeNode(n.id)">删除</button>
              </template>
              <template v-else>
                <button class="ghost" title="把该策略组加入其它分组（作为伪节点）" @click="openJoin(n)">＋入组</button>
                <button class="ghost danger" title="删除该策略组" @click="deleteStrategyNode(n)">✕策略</button>
              </template>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-if="!nodes.length" class="empty">当前分组没有节点 — 在上方粘贴导入，或在「订阅」页抓取保存。</div>
      <div v-else-if="!visibleNodes.length" class="empty">该组节点全部为 IPv6（已按设置过滤）— 可在「设置」关闭过滤。</div>
    </section>

    <!-- create strategy group -->
    <div v-if="stratOpen" class="dialog-mask" @click.self="stratOpen = false">
      <div class="dialog">
        <h3>新建策略组（成员分组自动选优）</h3>
        <div class="dialog-row"><label>名称</label><input v-model="stratName" class="inp" placeholder="如 自动选优" /></div>
        <label class="check" style="align-self: flex-start"><input v-model="stratAuto" type="checkbox" /> 自动（启动时测速缺失的成员，选最快节点）</label>
        <div class="node-pick">
          <label v-for="g in normalGroups" :key="g.id" class="pick">
            <input type="checkbox" v-model="stratMembers[g.id]" /> {{ g.name }}
          </label>
          <div v-if="!normalGroups.length" class="dim">没有可选成员分组</div>
        </div>
        <div class="dialog-btns">
          <span v-if="err" class="err">{{ err }}</span>
          <span class="spacer"></span>
          <button class="ghost" @click="stratOpen = false">取消</button>
          <button :disabled="stratBusy" @click="createStrategy">{{ stratBusy ? "创建中…" : "创建" }}</button>
        </div>
      </div>
    </div>

    <!-- join strategy into another group -->
    <div v-if="joinOf" class="dialog-mask" @click.self="joinOf = null">
      <div class="dialog">
        <h3>把「{{ joinOf.remark }}」加入分组</h3>
        <div class="dialog-row"><label>目标分组</label>
          <select v-model="joinTarget" class="inp sel">
            <option v-for="g in groups.filter((x) => (x.kind ?? 'normal') === 'normal' && x.id !== currentGroupId())" :key="g.id" :value="g.id">{{ g.name }}</option>
          </select>
        </div>
        <div class="dialog-btns">
          <span class="spacer"></span>
          <button class="ghost" @click="joinOf = null">取消</button>
          <button :disabled="joinTarget == null" @click="doJoin">加入</button>
        </div>
      </div>
    </div>

    <!-- QR share dialog -->
    <div v-if="qrModal" class="dialog-mask" @click.self="qrModal = null">
      <div class="dialog qr-dialog">
        <h3>{{ qrModal.remark }}</h3>
        <img :src="qrModal.dataUrl" alt="QR" class="qr-img" />
        <div class="qr-link" :title="qrModal.link">{{ qrModal.link }}</div>
        <div class="dialog-btns">
          <span class="spacer"></span>
          <button class="ghost" @click="qrModal = null">关闭</button>
          <button @click="copyQrLink">复制链接</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.servers-page { display: flex; flex-direction: column; gap: 12px; }
.toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
.toolbar h1 { margin: 0 8px 0 0; font-size: 20px; }
.spacer { flex: 1; }
button {
  border: 0; border-radius: 6px; background: var(--accent); color: #fff;
  padding: 6px 12px; font-size: 13px; cursor: pointer; white-space: nowrap;
}
button:disabled { opacity: 0.5; cursor: default; }
.ghost { background: transparent; border: 1px solid var(--border); color: inherit; }
.danger { color: #ef4444; border-color: rgba(239,68,68,0.4); }
.group-select {
  border: 1px solid var(--border); border-radius: 6px; padding: 5px 8px;
  background: transparent; color: inherit; font-size: 13px; max-width: 180px;
}
.group-ops { display: inline-flex; gap: 4px; }
button.mini { padding: 3px 8px; font-size: 12px; }
.chip { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; padding: 4px 8px; border-radius: 20px; border: 1px solid var(--border); }
.chip .dot { width: 8px; height: 8px; border-radius: 50%; background: #6b7280; }
.chip.on .dot { background: #22c55e; }
.chip.on { border-color: rgba(34,197,94,0.4); }
.proxy-toggle { display: inline-flex; align-items: center; gap: 5px; font-size: 13px; cursor: pointer; }
.err { color: #ef4444; font-size: 12px; }
.ok { color: #22c55e; font-size: 12px; }
.import-card { border: 1px solid var(--border); border-radius: 8px; padding: 10px; max-width: 820px; }
.row { display: flex; gap: 10px; align-items: flex-start; }
.row textarea {
  flex: 1; min-height: 44px; border: 1px solid var(--border); border-radius: 8px; padding: 6px;
  background: transparent; color: inherit; font-family: ui-monospace, monospace; font-size: 12px; resize: vertical;
}
.nodes { max-width: 860px; }
.sort-toggle { display: inline-flex; align-items: center; gap: 5px; font-size: 12px; cursor: pointer; }
.dim { opacity: 0.55; font-size: 11px; }
.msgbar {
  min-height: 18px; line-height: 18px; font-size: 12px;
  overflow: hidden; white-space: nowrap; text-overflow: ellipsis;
}
.msgbar .ok { color: #22c55e; }
.msgbar .err { color: #ef4444; }
.dialog-mask {
  position: fixed; inset: 0; background: rgba(0, 0, 0, 0.45);
  display: flex; align-items: center; justify-content: center; z-index: 50;
}
.dialog {
  background: var(--bg, #f5f6f8); color: var(--fg, #1f2329);
  border: 1px solid var(--border); border-radius: 12px; padding: 16px;
  width: min(380px, 92vw); display: flex; flex-direction: column; align-items: center; gap: 10px;
}
.dialog h3 { margin: 0; font-size: 14px; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.qr-img { width: 300px; height: 300px; image-rendering: pixelated; background: #fff; padding: 6px; border-radius: 8px; }
.qr-link {
  width: 100%; font-family: ui-monospace, monospace; font-size: 10px; word-break: break-all;
  border: 1px solid var(--border); border-radius: 6px; padding: 6px; max-height: 60px; overflow: auto;
}
.dialog-btns { display: flex; width: 100%; gap: 8px; justify-content: flex-end; }
.dialog-btns .spacer { flex: 1; }
.nodes-head { display: flex; align-items: center; padding: 4px 2px; font-size: 13px; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; padding: 6px 8px; border-top: 1px solid var(--border); }
th { font-size: 11px; text-transform: uppercase; opacity: 0.7; }
tr.current td { background: rgba(59,130,246,0.08); }
.sel { cursor: pointer; width: 28px; text-align: center; color: var(--accent); }
.remark { cursor: pointer; }
.ops { white-space: nowrap; text-align: right; }
.ops button { padding: 2px 8px; font-size: 12px; margin-left: 4px; }
.bad { color: #ef4444; }
code { font-size: 11px; }
.empty { padding: 18px; text-align: center; opacity: 0.55; font-size: 13px; }
</style>
