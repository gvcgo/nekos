<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  coreStatus,
  coreStop,
  coreStart,
  createGroup,
  deleteGroup,
  deleteNode,
  groupsList,
  importToGroup,
  latencyList,
  measureNode,
  nodesList,
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

/** latency ordering: measured asc; failed after measured; untested last. */
const sortedNodes = computed(() => {
  const list = nodes.value;
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

/** Run async work over items with bounded concurrency. */
async function mapLimit<T>(
  items: T[],
  limit: number,
  worker: (item: T) => Promise<void>,
): Promise<void> {
  let idx = 0;
  const lanes: Promise<void>[] = [];
  for (let lane = 0; lane < Math.min(limit, items.length); lane++) {
    lanes.push(
      (async () => {
        while (idx < items.length) {
          const item = items[idx++];
          try {
            await worker(item);
          } catch (e) {
            err.value = String(e);
          }
        }
      })(),
    );
  }
  await Promise.all(lanes);
}

async function testAll() {
  testingAll.value = true;
  await mapLimit(nodes.value, 6, async (n) => {
    await testNode(n.id);
  });
  testingAll.value = false;
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
        <option v-for="g in groups" :key="g.id" :value="g.id">{{ g.name }}</option>
      </select>
      <span class="group-ops" title="管理分组">
        <button class="ghost mini" @click="newGroup">＋ 新建</button>
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
      <button :disabled="startBusy || !nodes.length" @click="toggleStart">
        {{ status.running ? "停止" : startBusy ? "启动中…" : "启动" }}
      </button>
    </div>

    <span v-if="switchMsg" class="ok">{{ switchMsg }}</span>
    <span v-if="err" class="err">{{ err }}</span>

    <section class="import-card">
      <div class="row">
        <textarea v-model="importText" rows="2" placeholder="粘贴分享链接 / 订阅内容（base64、Clash、JSON），导入到当前分组" />
        <button :disabled="importBusy || !importText.trim()" @click="doImport">{{ importBusy ? "导入中…" : "导入到当前分组" }}</button>
      </div>
      <span v-if="importMsg" class="ok">{{ importMsg }}</span>
    </section>

    <section class="nodes">
      <div class="nodes-head">
        <strong>节点（{{ nodes.length }}）</strong>
        <span class="spacer"></span>
        <label class="sort-toggle" title="按延迟升序排列（失败与未测在后）">
          <input type="checkbox" :checked="sortByDelay" @change="toggleSort(($event.target as HTMLInputElement).checked)" />
          按延迟排序
        </label>
        <button class="ghost" :disabled="testingAll || !nodes.length" @click="testAll">
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
              <button class="ghost danger" @click="removeNode(n.id)">删除</button>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-if="!nodes.length" class="empty">当前分组没有节点 — 在上方粘贴导入，或在「订阅」页抓取保存。</div>
    </section>
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
