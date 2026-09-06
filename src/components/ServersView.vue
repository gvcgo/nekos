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
import { useDict, fmt, type Dict } from "../i18n";

const zhL = {
  titleGroups: "分组",
  manageGroups: "管理分组",
  addGroup: "＋ 新建",
  addStrategy: "＋策略组",
  strategyTip: "按成员分组延迟自动选最快的组（v2rayN 策略组）",
  renameBtn: "✎ 改名",
  deleteBtn: "✕ 删除",
  deleteGroupTip: "删除分组（含节点）",
  chipRunOn: "运行中 · 系统代理开",
  chipRunOff: "运行中 · 系统代理关",
  chipStopped: "已停止",
  modeTip: "分流模式",
  modeGlobal: "全局代理",
  modeRule: "规则(绕过大陆)",
  modeDirect: "直连",
  proxyTip: "内核运行中可随时开关系统代理",
  sysProxy: "系统代理",
  start: "启动",
  stop: "停止",
  starting: "启动中…",
  importPlaceholder: "粘贴分享链接 / 订阅内容（base64、Clash、JSON），导入到当前分组",
  importBtn: "导入到当前分组",
  importing: "导入中…",
  importOk: "导入 {n} 个节点，错误 {e}",
  nodeCount: "节点（{n}）",
  hiddenV6: "已隐藏 {n} 个 IPv6",
  sortTip: "按延迟升序排列（失败与未测在后）",
  sortByDelay: "按延迟排序",
  testAll: "全部测速",
  testing: "测速中…",
  thType: "类型",
  thRemark: "备注",
  thLatency: "延迟",
  thOps: "操作",
  test: "测速",
  copy: "复制",
  copyTip: "复制分享链接",
  qrTip: "二维码分享",
  remove: "删除",
  joinIn: "＋入组",
  joinTip: "把该策略组加入其它分组（作为伪节点）",
  stratDel: "✕策略",
  stratDelTip: "删除该策略组",
  emptyNoNodes: "当前分组没有节点 — 在上方粘贴导入，或在「订阅」页抓取保存。",
  emptyAllV6: "该组节点全部为 IPv6（已按设置过滤）— 可在「设置」关闭过滤。",
  stratTitle: "新建策略组（成员分组自动选优）",
  nameLabel: "名称",
  namePlaceholder: "如 自动选优",
  autoCheck: "自动（启动时测速缺失的成员，选最快节点）",
  noMembers: "没有可选成员分组",
  cancel: "取消",
  creating: "创建中…",
  create: "创建",
  stratNeedName: "请填写名称并选择至少一个成员分组",
  joinTitle: "把「{remark}」加入分组",
  joinTargetLabel: "目标分组",
  joinNoGroups: "还没有其它分组 — 填名称直接新建一个并加入：",
  joinNewName: "新分组名称",
  joining: "加入中…",
  joinBtn: "加入",
  joinCreateAndAdd: "新建并加入",
  joinNeedTarget: "请选择目标分组，或填写新分组名称后加入",
  joinOk: "已将「{remark}」加入分组「{group}」",
  dupSuffix: "（已存在）",
  delStrategyConfirm: "删除策略组「{name}」？",
  delGroupConfirm: "删除分组「{name}」及其全部节点？",
  promptNewGroup: "新分组名称",
  promptRename: "重命名分组",
  qrCopied: "链接已复制到剪贴板",
  copyOk: "已复制「{remark}」链接",
  copyFail: "复制失败",
  copyFailManual: "复制失败（请使用 QR 弹窗内手动复制）",
  switchedProxy: "已切换代理到「{remark}」",
  proxyOnMsg: "系统代理已开启（关闭「停止」或再点开关即可还原）",
  close: "关闭",
  copyLink: "复制链接",
  latDash: "—",
  latFail: "失败",
  latMs: "{n} ms",
} as const;
type DictKeys = keyof typeof zhL;
const enL: Record<DictKeys, string> = {
  titleGroups: "Groups",
  manageGroups: "Manage groups",
  addGroup: "+ New",
  addStrategy: "+Strategy",
  strategyTip:
    "Auto-picks the fastest member group by per-member latency (v2rayN strategy group)",
  renameBtn: "✎ Rename",
  deleteBtn: "✕ Delete",
  deleteGroupTip: "Delete group (with its nodes)",
  chipRunOn: "Running · System proxy on",
  chipRunOff: "Running · System proxy off",
  chipStopped: "Stopped",
  modeTip: "Routing mode",
  modeGlobal: "Global",
  modeRule: "Rule (bypass mainland)",
  modeDirect: "Direct",
  proxyTip: "Toggle the system proxy at any time while the core is running",
  sysProxy: "System proxy",
  start: "Start",
  stop: "Stop",
  starting: "Starting…",
  importPlaceholder:
    "Paste share links / subscription content (base64, Clash, JSON) to import into the current group",
  importBtn: "Import to current group",
  importing: "Importing…",
  importOk: "Imported {n} node(s), {e} error(s)",
  nodeCount: "Nodes ({n})",
  hiddenV6: "{n} IPv6 hidden",
  sortTip: "Sort by latency ascending (failed & untested last)",
  sortByDelay: "Sort by latency",
  testAll: "Test all",
  testing: "Testing…",
  thType: "Type",
  thRemark: "Remark",
  thLatency: "Latency",
  thOps: "Actions",
  test: "Test",
  copy: "Copy",
  copyTip: "Copy share link",
  qrTip: "Share via QR code",
  remove: "Delete",
  joinIn: "+Join",
  joinTip: "Mount this strategy group into other groups (as a pseudo node)",
  stratDel: "✕Strategy",
  stratDelTip: "Delete this strategy group",
  emptyNoNodes:
    "No nodes in this group yet — paste a link above to import, or fetch in the Subscriptions page.",
  emptyAllV6:
    "All nodes in this group are IPv6 (filtered by settings) — disable the filter in Settings.",
  stratTitle: "New strategy group (auto-pick best members)",
  nameLabel: "Name",
  namePlaceholder: "e.g. auto-best",
  autoCheck:
    "Auto (test untested members at startup, pick the fastest node)",
  noMembers: "No member groups available",
  cancel: "Cancel",
  creating: "Creating…",
  create: "Create",
  stratNeedName: "Enter a name and select at least one member group",
  joinTitle: 'Add "{remark}" to a group',
  joinTargetLabel: "Target group",
  joinNoGroups: "No other groups yet — enter a name to create and join one:",
  joinNewName: "New group name",
  joining: "Joining…",
  joinBtn: "Join",
  joinCreateAndAdd: "Create & join",
  joinNeedTarget:
    "Select a target group, or enter a new group name to join",
  joinOk: 'Added "{remark}" to group "{group}"',
  dupSuffix: " (already exists)",
  delStrategyConfirm: 'Delete strategy group "{name}"?',
  delGroupConfirm: 'Delete group "{name}" and all its nodes?',
  promptNewGroup: "New group name",
  promptRename: "Rename group",
  qrCopied: "Link copied to clipboard",
  copyOk: 'Copied link for "{remark}"',
  copyFail: "Copy failed",
  copyFailManual: "Copy failed (use manual copy inside the QR dialog)",
  switchedProxy: 'Switched proxy to "{remark}"',
  proxyOnMsg: "System proxy enabled (press Stop or toggle again to restore)",
  close: "Close",
  copyLink: "Copy link",
  latDash: "—",
  latFail: "Failed",
  latMs: "{n} ms",
};
const arL: Record<DictKeys, string> = {
  titleGroups: "المجموعات",
  manageGroups: "إدارة المجموعات",
  addGroup: "＋ مجموعة جديدة",
  addStrategy: "＋ إستراتيجية",
  strategyTip:
    "اختيار أسرع مجموعة تلقائيًا حسب زمن استجابة أعضائها (مجموعة إستراتيجية v2rayN)",
  renameBtn: "✎ إعادة التسمية",
  deleteBtn: "✕ حذف",
  deleteGroupTip: "حذف المجموعة (بما تحويه من عقد)",
  chipRunOn: "يعمل · وكيل النظام مفعّل",
  chipRunOff: "يعمل · وكيل النظام معطّل",
  chipStopped: "متوقف",
  modeTip: "نمط التوجيه",
  modeGlobal: "وكيل شامل",
  modeRule: "قواعد (تجاوز البر الرئيسي للصين)",
  modeDirect: "مباشر",
  proxyTip: "يمكن تشغيل وكيل النظام أو إيقافه في أي وقت أثناء تشغيل النواة",
  sysProxy: "وكيل النظام",
  start: "تشغيل",
  stop: "إيقاف",
  starting: "جارٍ التشغيل…",
  importPlaceholder:
    "الصق روابط المشاركة / محتوى الاشتراكات (base64 أو Clash أو JSON) لاستيرادها إلى المجموعة الحالية",
  importBtn: "استيراد إلى المجموعة الحالية",
  importing: "جارٍ الاستيراد…",
  importOk: "تم استيراد {n} من العقد، مع {e} من الأخطاء",
  nodeCount: "العقد ({n})",
  hiddenV6: "تم إخفاء {n} من عقد IPv6",
  sortTip: "الترتيب حسب زمن الاستجابة تصاعديًا (الفاشلة وغير المختبرة في النهاية)",
  sortByDelay: "الترتيب حسب زمن الاستجابة",
  testAll: "قياس الكل",
  testing: "جارٍ القياس…",
  thType: "النوع",
  thRemark: "الملاحظة",
  thLatency: "زمن الاستجابة",
  thOps: "إجراءات",
  test: "قياس",
  copy: "نسخ",
  copyTip: "نسخ رابط المشاركة",
  qrTip: "مشاركة عبر رمز QR",
  remove: "حذف",
  joinIn: "＋ ضمّ",
  joinTip: "ضمّ هذه المجموعة الإستراتيجية إلى مجموعات أخرى (كعقدة وهمية)",
  stratDel: "✕ إستراتيجية",
  stratDelTip: "حذف هذه المجموعة الإستراتيجية",
  emptyNoNodes:
    "لا توجد عقد في المجموعة الحالية — الصق رابطًا أعلاه للاستيراد، أو اجلبها من صفحة «الاشتراكات».",
  emptyAllV6:
    "جميع عقد هذه المجموعة من IPv6 (مُصفاة حسب الإعدادات) — يمكنك إيقاف التصفية من «الإعدادات».",
  stratTitle: "مجموعة إستراتيجية جديدة (اختيار أفضل الأعضاء تلقائيًا)",
  nameLabel: "الاسم",
  namePlaceholder: "مثال: الأفضل تلقائيًا",
  autoCheck: "تلقائي (قياس الأعضاء غير المُقاسَة عند التشغيل واختيار الأسرع منها)",
  noMembers: "لا توجد مجموعات أعضاء متاحة",
  cancel: "إلغاء",
  creating: "جارٍ الإنشاء…",
  create: "إنشاء",
  stratNeedName: "أدخل اسمًا واختر مجموعة عضو واحدة على الأقل",
  joinTitle: "ضمّ «{remark}» إلى مجموعة",
  joinTargetLabel: "المجموعة الهدف",
  joinNoGroups: "لا توجد مجموعات أخرى بعد — أدخل اسمًا لإنشاء مجموعة جديدة والضم إليها:",
  joinNewName: "اسم المجموعة الجديدة",
  joining: "جارٍ الضم…",
  joinBtn: "ضمّ",
  joinCreateAndAdd: "إنشاء والضم",
  joinNeedTarget: "اختر مجموعة هدف، أو أدخل اسم مجموعة جديدة للضم إليها",
  joinOk: "تمت إضافة «{remark}» إلى المجموعة «{group}»",
  dupSuffix: " (موجودة مسبقًا)",
  delStrategyConfirm: "حذف المجموعة الإستراتيجية «{name}»؟",
  delGroupConfirm: "حذف المجموعة «{name}» وكل عقدها؟",
  promptNewGroup: "اسم المجموعة الجديدة",
  promptRename: "إعادة تسمية المجموعة",
  qrCopied: "تم نسخ الرابط إلى الحافظة",
  copyOk: "تم نسخ رابط «{remark}»",
  copyFail: "فشل النسخ",
  copyFailManual: "فشل النسخ (استخدم النسخ اليدوي داخل نافذة رمز QR)",
  switchedProxy: "تم تحويل الوكيل إلى «{remark}»",
  proxyOnMsg: "تم تفعيل وكيل النظام (أوقفه بزر «إيقاف» أو بدّل الخيار للتراجع)",
  close: "إغلاق",
  copyLink: "نسخ الرابط",
  latDash: "—",
  latFail: "فشل",
  latMs: "{n} ms",
};
const dict = useDict({ zh: zhL as Dict, en: enL, ar: arL });
function tt(k: DictKeys, p?: Record<string, string | number>): string {
  return fmt(dict.value[k] as string, p);
}

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
    importMsg.value = tt("importOk", { n: res.nodes.length, e: res.errors.length });
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
      err.value = tt("stratNeedName");
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
  if (!sid || !confirm(tt("delStrategyConfirm", { name: n.remark }))) return;
  await deleteGroup(sid);
  await reloadNodes();
}

// join (mount) a strategy node into another group
const joinOf = ref<Node | null>(null);
const joinTarget = ref<number | null>(null);
const newJoinName = ref("");
const joinBusy = ref(false);

const joinableGroups = computed(() =>
  groups.value.filter(
    (g) => (g.kind ?? "normal") === "normal" && g.id !== currentGroupId(),
  ),
);

function openJoin(n: Node) {
  joinOf.value = n;
  newJoinName.value = "";
  err.value = "";
  joinTarget.value = joinableGroups.value[0]?.id ?? null;
}

async function doJoin() {
  const n = joinOf.value;
  if (!n) return;
  joinBusy.value = true;
  err.value = "";
  shareMsg.value = "";
  try {
    let target = joinTarget.value;
    if (target == null) {
      // create a fresh group as the target
      const name = newJoinName.value.trim();
      if (!name) {
        err.value = tt("joinNeedTarget");
        return;
      }
      const created = await createGroup(name);
      target = created.id;
    }
    const out = await copyNodes(currentGroupId(), target, [n.id]);
    const targetName =
      groups.value.find((g) => g.id === target)?.name ??
      newJoinName.value.trim() ??
      "";
    shareMsg.value =
      tt("joinOk", { remark: n.remark, group: targetName }) +
      (out.duplicated ? tt("dupSuffix") : "");
    joinOf.value = null;
    if (out.inserted > 0) await loadAll();
  } catch (e) {
    err.value = String(e);
  } finally {
    joinBusy.value = false;
  }
}

async function removeGroup() {
  const gid = currentGroupId();
  if (gid === 1) return;
  if (!confirm(tt("delGroupConfirm", { name: currentGroup().name }))) return;
  await deleteGroup(gid);
  await loadAll();
}

async function newGroup() {
  const name = prompt(tt("promptNewGroup"), "");
  if (!name?.trim()) return;
  const g = await createGroup(name.trim());
  await loadAll();
  if (g.id) await switchGroup(g.id);
}

async function renameCurrentGroup() {
  const gid = currentGroupId();
  if (gid === 1) return;
  const name = prompt(tt("promptRename"), currentGroup().name);
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
  shareMsg.value = ok ? tt("qrCopied") : tt("copyFail");
}

async function copyNodeLink(groupId: number, nodeId: string, remark: string) {
  shareMsg.value = "";
  err.value = "";
  try {
    const link = await nodeEncode(groupId, nodeId);
    const ok = await copyText(link);
    shareMsg.value = ok ? tt("copyOk", { remark }) : tt("copyFailManual");
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
      switchMsg.value = tt("switchedProxy", {
        remark: nodes.value.find((n) => n.id === nodeId)?.remark ?? nodeId,
      });
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
    if (on) switchMsg.value = tt("proxyOnMsg");
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
  if (!m) return tt("latDash");
  if (m.delay_ms != null) return tt("latMs", { n: m.delay_ms });
  if (m.error) return tt("latFail");
  return tt("testing"); // probe in flight (no result yet)
}

onMounted(loadAll);
</script>

<template>
  <div class="servers-page">
    <div class="toolbar">
      <div class="trow">
        <h1>{{ tt("titleGroups") }}</h1>
        <select class="group-select" :value="currentGroupId()" @change="switchGroup(Number(($event.target as HTMLSelectElement).value))">
          <option v-for="g in groups.filter((x) => (x.kind ?? 'normal') === 'normal')" :key="g.id" :value="g.id">{{ g.name }}</option>
        </select>
        <span class="group-ops" :title="tt('manageGroups')">
          <button class="ghost mini" @click="newGroup">{{ tt("addGroup") }}</button>
          <button class="ghost mini" :title="tt('strategyTip')" @click="openStrategyDialog">{{ tt("addStrategy") }}</button>
          <button v-if="currentGroupId() !== 1" class="ghost mini" @click="renameCurrentGroup">{{ tt("renameBtn") }}</button>
          <button v-if="currentGroupId() !== 1" class="ghost mini danger" :title="tt('deleteGroupTip')" @click="removeGroup">{{ tt("deleteBtn") }}</button>
        </span>
        <span class="spacer"></span>
      </div>
      <div class="trow trow-ops">
        <label class="chip" :class="status.running ? 'on' : 'off'">
          <span class="dot"></span>
          {{ status.running ? (status.proxy_enabled ? tt("chipRunOn") : tt("chipRunOff")) : tt("chipStopped") }}
        </label>
        <select class="group-select" :value="settings?.mode ?? 'global'" :title="tt('modeTip')" @change="changeMode(($event.target as HTMLSelectElement).value)">
          <option value="global">{{ tt("modeGlobal") }}</option>
          <option value="rule">{{ tt("modeRule") }}</option>
          <option value="direct">{{ tt("modeDirect") }}</option>
        </select>
        <label class="proxy-toggle" :title="tt('proxyTip')">
          <input type="checkbox" :checked="status.proxy_enabled" :disabled="!status.running" @change="toggleProxy(($event.target as HTMLInputElement).checked)" />
          {{ tt("sysProxy") }}
        </label>
        <button class="primary" :disabled="startBusy || !visibleNodes.length" @click="toggleStart">
          {{ status.running ? tt("stop") : startBusy ? tt("starting") : tt("start") }}
        </button>
        <span class="spacer"></span>
      </div>
    </div>

    <div class="msgbar">
      <span v-if="switchMsg" class="ok">{{ switchMsg }}</span>
      <span v-if="shareMsg" class="ok">{{ shareMsg }}</span>
      <span v-if="err" class="err">{{ err }}</span>
    </div>

    <section v-if="!isVirtualCurrent" class="import-card">
      <div class="row">
        <textarea v-model="importText" rows="2" :placeholder="tt('importPlaceholder')" />
        <button :disabled="importBusy || !importText.trim()" @click="doImport">{{ importBusy ? tt("importing") : tt("importBtn") }}</button>
      </div>
      <span v-if="importMsg" class="ok">{{ importMsg }}</span>
    </section>

    <section class="nodes">
      <div class="nodes-head">
        <strong>{{ tt("nodeCount", { n: visibleNodes.length }) }}</strong>
        <span v-if="settings?.filter_ipv6 && visibleNodes.length < nodes.length" class="dim">
          {{ tt("hiddenV6", { n: nodes.length - visibleNodes.length }) }}
        </span>
        <span class="spacer"></span>
        <label class="sort-toggle" :title="tt('sortTip')">
          <input type="checkbox" :checked="sortByDelay" @change="toggleSort(($event.target as HTMLInputElement).checked)" />
          {{ tt("sortByDelay") }}
        </label>
        <button class="ghost" :disabled="testingAll || !visibleNodes.length" @click="testAll">
          {{ testingAll ? tt("testing") : tt("testAll") }}
        </button>
      </div>
      <table>
        <thead>
          <tr><th></th><th>{{ tt("thType") }}</th><th>{{ tt("thRemark") }}</th><th>{{ tt("thLatency") }}</th><th>{{ tt("thOps") }}</th></tr>
        </thead>
        <tbody>
          <tr v-for="n in sortedNodes" :key="n.id" :class="{ current: n.id === selectedNodeId() }">
            <td class="sel" @click="pickNode(n.id)">{{ n.id === selectedNodeId() ? "●" : "○" }}</td>
            <td><code>{{ n.type }}</code></td>
            <td class="remark" @click="pickNode(n.id)">{{ n.remark }}</td>
            <td :class="delayMap[n.id]?.error ? 'bad' : ''">{{ delayText(n) }}</td>
            <td class="ops">
              <button class="ghost" :disabled="!!delayMap[n.id] && delayMap[n.id]!.delay_ms == null && !delayMap[n.id]!.error" @click="testNode(n.id)">{{ tt("test") }}</button>
              <template v-if="n.type !== 'strategy'">
                <button class="ghost" :disabled="qrBusy" :title="tt('copyTip')" @click="copyNodeLink(n.group_id, n.id, n.remark)">{{ tt("copy") }}</button>
                <button class="ghost" :title="tt('qrTip')" @click="showNodeQr(n.group_id, n.id, n.remark)">QR</button>
                <button v-if="!isVirtualCurrent" class="ghost danger" @click="removeNode(n.id)">{{ tt("remove") }}</button>
              </template>
              <template v-else>
                <button class="ghost" :title="tt('joinTip')" @click="openJoin(n)">{{ tt("joinIn") }}</button>
                <button class="ghost danger" :title="tt('stratDelTip')" @click="deleteStrategyNode(n)">{{ tt("stratDel") }}</button>
              </template>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-if="!nodes.length" class="empty">{{ tt("emptyNoNodes") }}</div>
      <div v-else-if="!visibleNodes.length" class="empty">{{ tt("emptyAllV6") }}</div>
    </section>

    <!-- create strategy group -->
    <div v-if="stratOpen" class="dialog-mask" @click.self="stratOpen = false">
      <div class="dialog">
        <h3>{{ tt("stratTitle") }}</h3>
        <div class="dialog-row"><label>{{ tt("nameLabel") }}</label><input v-model="stratName" class="inp" :placeholder="tt('namePlaceholder')" /></div>
        <label class="check" style="align-self: flex-start"><input v-model="stratAuto" type="checkbox" /> {{ tt("autoCheck") }}</label>
        <div class="node-pick">
          <label v-for="g in normalGroups" :key="g.id" class="pick">
            <input type="checkbox" v-model="stratMembers[g.id]" /> {{ g.name }}
          </label>
          <div v-if="!normalGroups.length" class="dim">{{ tt("noMembers") }}</div>
        </div>
        <div class="dialog-btns">
          <span v-if="err" class="err">{{ err }}</span>
          <span class="spacer"></span>
          <button class="ghost" @click="stratOpen = false">{{ tt("cancel") }}</button>
          <button :disabled="stratBusy" @click="createStrategy">{{ stratBusy ? tt("creating") : tt("create") }}</button>
        </div>
      </div>
    </div>

    <!-- join strategy into another group -->
    <div v-if="joinOf" class="dialog-mask" @click.self="joinOf = null">
      <div class="dialog">
        <h3>{{ tt("joinTitle", { remark: joinOf.remark }) }}</h3>
        <template v-if="joinableGroups.length">
          <div class="dialog-row"><label>{{ tt("joinTargetLabel") }}</label>
            <select v-model="joinTarget" class="inp sel">
              <option v-for="g in joinableGroups" :key="g.id" :value="g.id">{{ g.name }}</option>
            </select>
          </div>
        </template>
        <template v-else>
          <div class="dim" style="align-self: flex-start">{{ tt("joinNoGroups") }}</div>
          <input v-model="newJoinName" class="inp" :placeholder="tt('joinNewName')" style="align-self: stretch" />
        </template>
        <div class="dialog-btns">
          <span v-if="err" class="err">{{ err }}</span>
          <span class="spacer"></span>
          <button class="ghost" @click="joinOf = null">{{ tt("cancel") }}</button>
          <button :disabled="joinBusy || (joinTarget == null && !newJoinName.trim())" @click="doJoin">
            {{ joinBusy ? tt("joining") : joinTarget != null ? tt("joinBtn") : tt("joinCreateAndAdd") }}
          </button>
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
          <button class="ghost" @click="qrModal = null">{{ tt("close") }}</button>
          <button @click="copyQrLink">{{ tt("copyLink") }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.servers-page { display: flex; flex-direction: column; gap: 12px; }
.toolbar { display: flex; flex-direction: column; gap: 8px; }
.trow { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
.trow-ops { gap: 12px; }
.trow h1 { margin: 0 8px 0 0; font-size: 20px; }
button.primary {
  border: 0; border-radius: 6px; background: var(--accent); color: #fff;
  padding: 6px 16px; font-size: 13px; cursor: pointer; white-space: nowrap;
}
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
th, td { text-align: start; padding: 6px 8px; border-top: 1px solid var(--border); }
th { font-size: 11px; text-transform: uppercase; opacity: 0.7; }
tr.current td { background: rgba(59,130,246,0.08); }
.sel { cursor: pointer; width: 28px; text-align: center; color: var(--accent); }
.remark { cursor: pointer; }
.ops { white-space: nowrap; text-align: end; }
.ops button { padding: 2px 8px; font-size: 12px; margin-left: 4px; }
.bad { color: #ef4444; }
code { font-size: 11px; }
.empty { padding: 18px; text-align: center; opacity: 0.55; font-size: 13px; }
</style>
