<script setup lang="ts">
// Route settings card for the Settings page: pick which routing profile is
// active in rule mode, and manage custom profiles. Rules are sing-box-native
// route rule JSON stored verbatim (same philosophy as node.out).
import { computed, onMounted, ref } from "vue";
import {
  routeProfileCreate,
  routeProfileDelete,
  routeProfileSetActive,
  routeProfileUpdate,
  routeProfilesList,
  settingsGet,
  type RouteProfile,
} from "../api";
import { fmt, locale } from "../i18n";

const zhL = {
  cardTitle: "路由设置",
  cardNote: "仅在「规则」模式下生效；内核运行中需在分组页重新「启动」。",
  activeLabel: "生效路由",
  builtin: "内置：绕过大陆",
  manageBtn: "管理路由档案…",
  manageTitle: "路由档案",
  noProfiles: "还没有自定义档案 — 先「新建档案」",
  newProfile: "＋ 新建档案",
  deleteProfile: "删除档案",
  delConfirm: "删除路由档案「{name}」？",
  nameLabel: "名称",
  finalLabel: "兜底出口",
  fProxy: "代理（选中的节点）",
  fDirect: "直连",
  fBlock: "阻止",
  rulesTitle: "规则（按序匹配，先中先停）",
  emptyRules: "还没有规则 — 点「＋ 规则」添加",
  addRule: "＋ 规则",
  editRule: "编辑",
  up: "上移",
  down: "下移",
  del: "删除",
  saveProfile: "保存档案",
  saveOk: "已保存",
  saving: "保存中…",
  errNeedName: "请输入档案名称",
  errNeedMatch: "规则至少需要一个匹配条件（域名/IP/规则集/端口/网络/协议）",
  editRuleTitle: "编辑规则",
  addRuleTitle: "添加规则",
  condDomainSuffix: "域名后缀",
  condDomainKeyword: "域名关键词",
  condDomainRegex: "域名正则",
  condIpCidr: "IP 段（CIDR）",
  condPort: "端口（如 443、8000-9000）",
  condNetwork: "网络",
  netAny: "任意",
  netTcp: "TCP",
  netUdp: "UDP",
  condProtocol: "嗅探协议",
  protoAny: "任意",
  protoTls: "TLS",
  protoHttp: "HTTP",
  protoQuic: "QUIC",
  protoUdp: "UDP",
  condSetGeoip: "中国 IP（geoip-cn）",
  condSetGeosite: "中国域名（geosite-cn）",
  invert: "取反（条件不匹配时命中）",
  outboundLabel: "动作",
  condPlace: "多个值用逗号、空格或换行分隔",
  cancel: "取消",
  add: "添加",
  saveRule: "保存",
  sSuffix: "后缀",
  sKeyword: "关键词",
  sRegex: "正则",
  sIp: "IP",
  sPort: "端口",
  sProto: "协议",
  sSet: "规则集",
} as const;

type DictKeys = keyof typeof zhL;
const enL: Record<DictKeys, string> = {
  cardTitle: "Routing",
  cardNote: "Applies in Rule mode only; restart the core on the Groups page to apply.",
  activeLabel: "Active route",
  builtin: "Built-in: bypass mainland China",
  manageBtn: "Manage route profiles…",
  manageTitle: "Route Profiles",
  noProfiles: "No custom profiles yet — use \"＋ New profile\"",
  newProfile: "＋ New profile",
  deleteProfile: "Delete profile",
  delConfirm: "Delete route profile \"{name}\"?",
  nameLabel: "Name",
  finalLabel: "Default outbound",
  fProxy: "Proxy (selected node)",
  fDirect: "Direct",
  fBlock: "Block",
  rulesTitle: "Rules (matched in order, first hit wins)",
  emptyRules: "No rules yet — use \"＋ Rule\" to add one",
  addRule: "＋ Rule",
  editRule: "Edit",
  up: "Move up",
  down: "Move down",
  del: "Delete",
  saveProfile: "Save profile",
  saveOk: "Saved",
  saving: "Saving…",
  errNeedName: "Enter a profile name",
  errNeedMatch: "A rule needs at least one condition (domain/IP/rule-set/port/network/protocol)",
  editRuleTitle: "Edit rule",
  addRuleTitle: "Add rule",
  condDomainSuffix: "Domain suffix",
  condDomainKeyword: "Domain keyword",
  condDomainRegex: "Domain regex",
  condIpCidr: "IP (CIDR)",
  condPort: "Ports (e.g. 443, 8000-9000)",
  condNetwork: "Network",
  netAny: "Any",
  netTcp: "TCP",
  netUdp: "UDP",
  condProtocol: "Sniffed protocol",
  protoAny: "Any",
  protoTls: "TLS",
  protoHttp: "HTTP",
  protoQuic: "QUIC",
  protoUdp: "UDP",
  condSetGeoip: "China IP (geoip-cn)",
  condSetGeosite: "China domains (geosite-cn)",
  invert: "Invert (match when the condition does NOT apply)",
  outboundLabel: "Action",
  condPlace: "Separate values with commas, spaces or newlines",
  cancel: "Cancel",
  add: "Add",
  saveRule: "Save",
  sSuffix: "suffix",
  sKeyword: "keyword",
  sRegex: "regex",
  sIp: "ip",
  sPort: "port",
  sProto: "protocol",
  sSet: "rule-set",
};

const dict = computed<Record<string, string>>(() => (locale.value === "en" ? enL : zhL));
function tt(k: DictKeys, p?: Record<string, string | number>): string {
  return fmt(dict.value[k], p);
}

type Obj = Record<string, unknown>;
type RuleRow = { key: number; obj: Obj };

let seq = 1;
const profiles = ref<RouteProfile[]>([]);
const activeId = ref<number | null>(null); // null => built-in bypass-mainland
const err = ref("");
const okMsg = ref("");
const manageOpen = ref(false);
const saving = ref(false);
const builtinKey = -1;

// ---- profile under edit (manager) ----
const selId = ref<number | null>(null); // null => unsaved new profile
const editName = ref("");
const editFinal = ref("proxy");
const editRules = ref<RuleRow[]>([]);

// ---- rule under edit (modal) ----
const ruleIdx = ref<number>(-1); // -1 => append
const ruleOpen = ref(false);
const ruleErr = ref("");
// rule condition inputs
const rSuffix = ref("");
const rKeyword = ref("");
const rRegex = ref("");
const rIp = ref("");
const rPort = ref("");
const rNetwork = ref("");
const rProtocol = ref("");
const rGeoip = ref(false);
const rGeosite = ref(false);
const rInvert = ref(false);
const rOut = ref("proxy");

function splitList(s: string): string[] {
  return s
    .split(/[\s,，、;；\n]+/)
    .map((x) => x.trim())
    .filter(Boolean);
}

function refreshProfiles(): Promise<void> {
  return routeProfilesList()
    .then((p) => {
      profiles.value = p;
    })
    .catch((e) => {
      err.value = String(e);
    });
}

async function refresh() {
  await refreshProfiles();
  const s = await settingsGet().catch(() => null);
  if (s) {
    activeId.value = s.route_profile_id ?? null;
  }
}

function loadProfile(p: RouteProfile) {
  selId.value = p.id;
  editName.value = p.name;
  editFinal.value = p.final_out;
  try {
    const arr = JSON.parse(p.rules_json) as Obj[];
    editRules.value = (Array.isArray(arr) ? arr : []).map((obj) => ({ key: seq++, obj }));
  } catch {
    editRules.value = [];
  }
}

function startNew() {
  selId.value = null;
  editName.value = "";
  editFinal.value = "proxy";
  editRules.value = [];
}

function openManage() {
  err.value = "";
  okMsg.value = "";
  manageOpen.value = true;
  const active = profiles.value.find((p) => p.id === activeId.value);
  if (active) loadProfile(active);
  else if (profiles.value.length) loadProfile(profiles.value[0]);
  else startNew();
}

function pickProfile(id: number) {
  const p = profiles.value.find((x) => x.id === id);
  if (p) loadProfile(p);
}

function moveRule(i: number, d: -1 | 1) {
  const j = i + d;
  if (j < 0 || j >= editRules.value.length) return;
  const arr = editRules.value;
  [arr[i], arr[j]] = [arr[j], arr[i]];
}

function removeRule(i: number) {
  editRules.value.splice(i, 1);
}

async function deleteProfile() {
  if (selId.value == null) return;
  const p = profiles.value.find((x) => x.id === selId.value);
  if (!p || !confirm(tt("delConfirm", { name: p.name }))) return;
  try {
    await routeProfileDelete(p.id);
    profiles.value = profiles.value.filter((x) => x.id !== p.id);
    if (activeId.value === p.id) activeId.value = null;
    if (profiles.value.length) loadProfile(profiles.value[0]);
    else startNew();
    okMsg.value = "";
  } catch (e) {
    err.value = String(e);
  }
}

async function saveProfile() {
  if (!editName.value.trim()) {
    err.value = tt("errNeedName");
    return;
  }
  saving.value = true;
  err.value = "";
  okMsg.value = "";
  try {
    const d = {
      name: editName.value.trim(),
      final_out: editFinal.value,
      rules_json: JSON.stringify(editRules.value.map((r) => r.obj)),
    };
    let saved: RouteProfile;
    if (selId.value == null) {
      saved = await routeProfileCreate(d.name, d.final_out, d.rules_json);
      profiles.value.push(saved);
      profiles.value = [...profiles.value];
    } else {
      saved = await routeProfileUpdate(selId.value, d.name, d.final_out, d.rules_json);
      const i = profiles.value.findIndex((p) => p.id === saved.id);
      if (i >= 0) profiles.value[i] = saved;
    }
    loadProfile(saved);
    okMsg.value = tt("saveOk");
  } catch (e) {
    err.value = String(e);
  } finally {
    saving.value = false;
  }
}

async function setActive(id: number | null) {
  try {
    await routeProfileSetActive(id);
    activeId.value = id;
    err.value = "";
  } catch (e) {
    err.value = String(e);
  }
}

function summarize(r: Obj): string {
  const parts: string[] = [];
  if (r.invert) parts.push("NOT");
  const push = (key: string, label: string) => {
    const vs = Array.isArray(r[key]) ? (r[key] as unknown[]).map(String) : [];
    if (!vs.length) return;
    const shown = vs.slice(0, 3).join(", ");
    parts.push(`${label}: ${shown}${vs.length > 3 ? ` +${vs.length - 3}` : ""}`);
  };
  push("domain_suffix", tt("sSuffix"));
  push("domain_keyword", tt("sKeyword"));
  push("domain_regex", tt("sRegex"));
  push("ip_cidr", tt("sIp"));
  push("port", tt("sPort"));
  if (r.protocol) parts.push(`${tt("sProto")}: ${String(r.protocol)}`);
  const sets = Array.isArray(r.rule_set) ? (r.rule_set as unknown[]).map(String) : [];
  if (sets.length) parts.push(`${tt("sSet")}: ${sets.join(", ")}`);
  const out =
    r.outbound === "block" ? "BLOCK" : r.outbound === "direct" ? "DIRECT" : "proxy";
  return `${parts.length ? parts.join(" · ") : "—"}  →  ${out}`;
}

// ---- rule modal ----

function openRule(idx: number) {
  ruleErr.value = "";
  if (idx >= 0 && idx < editRules.value.length) {
    const o = editRules.value[idx].obj;
    rSuffix.value = Array.isArray(o.domain_suffix) ? o.domain_suffix.join(", ") : "";
    rKeyword.value = Array.isArray(o.domain_keyword) ? o.domain_keyword.join(", ") : "";
    rRegex.value = Array.isArray(o.domain_regex) ? o.domain_regex.join(", ") : "";
    rIp.value = Array.isArray(o.ip_cidr) ? o.ip_cidr.join(", ") : "";
    rPort.value = Array.isArray(o.port) ? o.port.join(", ") : "";
    rNetwork.value = typeof o.network === "string" ? o.network : "";
    rProtocol.value = typeof o.protocol === "string" ? o.protocol : "";
    const sets = Array.isArray(o.rule_set) ? (o.rule_set as string[]) : [];
    rGeoip.value = sets.includes("geoip-cn");
    rGeosite.value = sets.includes("geosite-cn");
    rInvert.value = !!o.invert;
    rOut.value = typeof o.outbound === "string" ? o.outbound : "proxy";
  } else {
    rSuffix.value = rKeyword.value = rRegex.value = rIp.value = rPort.value = "";
    rNetwork.value = "";
    rProtocol.value = "";
    rGeoip.value = rGeosite.value = rInvert.value = false;
    rOut.value = "proxy";
  }
  ruleIdx.value = idx;
  ruleOpen.value = true;
}

function commitRule() {
  const obj: Obj = {};
  const set = (key: string, arr: string[]) => {
    if (arr.length) obj[key] = arr;
  };
  set("domain_suffix", splitList(rSuffix.value));
  set("domain_keyword", splitList(rKeyword.value));
  set("domain_regex", splitList(rRegex.value));
  set("ip_cidr", splitList(rIp.value));
  set("port", splitList(rPort.value));
  if (rNetwork.value) obj.network = rNetwork.value;
  if (rProtocol.value) obj.protocol = rProtocol.value;
  const sets: string[] = [];
  if (rGeoip.value) sets.push("geoip-cn");
  if (rGeosite.value) sets.push("geosite-cn");
  if (sets.length) obj.rule_set = sets;
  if (rInvert.value) obj.invert = true;
  const hasMatch =
    ["domain_suffix", "domain_keyword", "domain_regex", "ip_cidr", "port"].some(
      (k) => Array.isArray(obj[k]) && obj[k].length,
    ) || !!rNetwork.value || !!rProtocol.value || sets.length > 0;
  if (!hasMatch) {
    ruleErr.value = tt("errNeedMatch");
    return;
  }
  obj.outbound = rOut.value;
  if (ruleIdx.value >= 0 && ruleIdx.value < editRules.value.length) {
    editRules.value[ruleIdx.value].obj = obj;
  } else {
    editRules.value.push({ key: seq++, obj });
  }
  ruleOpen.value = false;
}

onMounted(refresh);
</script>

<template>
  <div class="route-card">
    <div class="card-head">
      <strong>{{ tt("cardTitle") }}</strong>
      <button class="ghost" @click="openManage">{{ tt("manageBtn") }}</button>
    </div>
    <label>{{ tt("activeLabel") }}</label>
    <select
      class="field"
      :value="activeId == null ? builtinKey : activeId"
      @change="setActive(Number(($event.target as HTMLSelectElement).value) === builtinKey ? null : Number(($event.target as HTMLSelectElement).value))"
    >
      <option :value="builtinKey">{{ tt("builtin") }}</option>
      <option v-for="p in profiles" :key="p.id" :value="p.id">{{ p.name }}</option>
    </select>
    <p class="note">{{ tt("cardNote") }}</p>

    <!-- profile manager -->
    <div v-if="manageOpen" class="dialog-mask" @click.self="manageOpen = false">
      <div class="dialog wide">
        <h3>{{ tt("manageTitle") }}</h3>
        <div class="layout">
          <div class="profiles">
            <button class="ghost" @click="startNew">{{ tt("newProfile") }}</button>
            <button
              v-for="p in profiles"
              :key="p.id"
              class="prof"
              :class="{ on: p.id === selId }"
              @click="pickProfile(p.id)"
            >
              {{ p.name }}
            </button>
            <div v-if="!profiles.length" class="dim">{{ tt("noProfiles") }}</div>
          </div>
          <div class="editor">
            <div class="row2">
              <label>{{ tt("nameLabel") }}</label>
              <input v-model="editName" class="inp" />
            </div>
            <div class="row2">
              <label>{{ tt("finalLabel") }}</label>
              <select v-model="editFinal" class="inp">
                <option value="proxy">{{ tt("fProxy") }}</option>
                <option value="direct">{{ tt("fDirect") }}</option>
                <option value="block">{{ tt("fBlock") }}</option>
              </select>
            </div>

            <div class="rules-head">
              <strong>{{ tt("rulesTitle") }}</strong>
              <button class="ghost" @click="openRule(-1)">{{ tt("addRule") }}</button>
            </div>
            <div v-if="!editRules.length" class="dim">{{ tt("emptyRules") }}</div>
            <table v-else class="rules">
              <tbody>
                <tr v-for="(r, i) in editRules" :key="r.key">
                  <td class="sum" @click="openRule(i)">{{ summarize(r.obj) }}</td>
                  <td class="ops">
                    <button class="mini" :title="tt('up')" @click="moveRule(i, -1)">↑</button>
                    <button class="mini" :title="tt('down')" @click="moveRule(i, 1)">↓</button>
                    <button class="mini" @click="openRule(i)">{{ tt("editRule") }}</button>
                    <button class="mini danger" @click="removeRule(i)">{{ tt("del") }}</button>
                  </td>
                </tr>
              </tbody>
            </table>

            <div class="btns">
              <span v-if="err" class="err">{{ err }}</span>
              <span v-if="okMsg" class="ok">{{ okMsg }}</span>
              <span class="spacer"></span>
              <button v-if="selId != null" class="ghost danger" @click="deleteProfile">
                {{ tt("deleteProfile") }}
              </button>
              <button :disabled="saving" @click="saveProfile">
                {{ saving ? tt("saving") : tt("saveProfile") }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- rule editor -->
    <div v-if="ruleOpen" class="dialog-mask" @click.self="ruleOpen = false">
      <div class="dialog">
        <h3>{{ ruleIdx >= 0 ? tt("editRuleTitle") : tt("addRuleTitle") }}</h3>
        <div class="grid">
          <label>{{ tt("condDomainSuffix") }}</label>
          <input v-model="rSuffix" class="inp" :placeholder="tt('condPlace')" />
          <label>{{ tt("condDomainKeyword") }}</label>
          <input v-model="rKeyword" class="inp" :placeholder="tt('condPlace')" />
          <label>{{ tt("condDomainRegex") }}</label>
          <input v-model="rRegex" class="inp" :placeholder="tt('condPlace')" />
          <label>{{ tt("condIpCidr") }}</label>
          <input v-model="rIp" class="inp" :placeholder="tt('condPlace')" />
          <label>{{ tt("condPort") }}</label>
          <input v-model="rPort" class="inp" :placeholder="tt('condPlace')" />
          <label>{{ tt("condNetwork") }}</label>
          <select v-model="rNetwork" class="inp">
            <option value="">{{ tt("netAny") }}</option>
            <option value="tcp">{{ tt("netTcp") }}</option>
            <option value="udp">{{ tt("netUdp") }}</option>
          </select>
          <label>{{ tt("condProtocol") }}</label>
          <select v-model="rProtocol" class="inp">
            <option value="">{{ tt("protoAny") }}</option>
            <option value="tls">{{ tt("protoTls") }}</option>
            <option value="http">{{ tt("protoHttp") }}</option>
            <option value="quic">{{ tt("protoQuic") }}</option>
            <option value="udp">{{ tt("protoUdp") }}</option>
          </select>
        </div>
        <div class="sets">
          <label class="check"><input v-model="rGeoip" type="checkbox" /> {{ tt("condSetGeoip") }}</label>
          <label class="check"><input v-model="rGeosite" type="checkbox" /> {{ tt("condSetGeosite") }}</label>
          <label class="check"><input v-model="rInvert" type="checkbox" /> {{ tt("invert") }}</label>
        </div>
        <div class="row2">
          <label>{{ tt("outboundLabel") }}</label>
          <select v-model="rOut" class="inp">
            <option value="proxy">{{ tt("fProxy") }}</option>
            <option value="direct">{{ tt("fDirect") }}</option>
            <option value="block">{{ tt("fBlock") }}</option>
          </select>
        </div>
        <div class="btns">
          <span v-if="ruleErr" class="err">{{ ruleErr }}</span>
          <span class="spacer"></span>
          <button class="ghost" @click="ruleOpen = false">{{ tt("cancel") }}</button>
          <button @click="commitRule">{{ ruleIdx >= 0 ? tt("saveRule") : tt("add") }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.route-card {
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-size: 13px;
}
.route-card label { margin-top: 6px; }
.card-head { display: flex; align-items: center; justify-content: space-between; }
.card-head strong { font-size: 13px; }
.field {
  border: 1px solid var(--border); border-radius: 6px; padding: 6px 8px;
  background: transparent; color: inherit; font-size: 13px; max-width: 320px;
}
.note { font-size: 12px; opacity: 0.65; margin: 0; line-height: 1.5; }
.dim { opacity: 0.55; font-size: 12px; padding: 6px 0; }
button {
  border: 0; border-radius: 6px; background: var(--accent); color: #fff;
  padding: 6px 12px; font-size: 13px; cursor: pointer; white-space: nowrap;
}
button:disabled { opacity: 0.5; cursor: default; }
.ghost { background: transparent; border: 1px solid var(--border); color: inherit; }
.danger { color: #ef4444; border-color: rgba(239, 68, 68, 0.4); }
.dialog-mask {
  position: fixed; inset: 0; background: rgba(0, 0, 0, 0.45);
  display: flex; align-items: center; justify-content: center; z-index: 60;
}
.dialog {
  background: var(--bg, #f5f6f8); color: var(--fg, #1f2329);
  border: 1px solid var(--border); border-radius: 12px; padding: 16px;
  width: min(680px, 92vw); display: flex; flex-direction: column; gap: 10px;
}
.dialog.wide { width: min(860px, 94vw); }
.dialog h3 { margin: 0; font-size: 15px; }
.layout { display: flex; gap: 14px; min-height: 320px; }
.profiles {
  flex: 0 0 200px; border-right: 1px solid var(--border); padding-right: 10px;
  display: flex; flex-direction: column; gap: 6px; align-items: stretch;
}
.profiles .prof {
  background: transparent; border: 1px solid transparent; color: inherit;
  text-align: start; padding: 5px 8px; font-size: 13px;
}
.profiles .prof.on { border-color: var(--accent); color: var(--accent); }
.editor { flex: 1; display: flex; flex-direction: column; gap: 8px; min-width: 0; }
.row2 { display: flex; align-items: center; gap: 10px; }
.row2 label { flex: 0 0 80px; font-size: 12px; opacity: 0.85; margin: 0; }
.inp {
  border: 1px solid var(--border); border-radius: 6px; padding: 5px 8px;
  background: transparent; color: inherit; font-size: 13px; width: 100%;
  font-family: inherit;
}
.rules-head { display: flex; align-items: center; justify-content: space-between; }
.rules { width: 100%; border-collapse: collapse; font-size: 12px; }
.rules td { padding: 4px 6px; border-top: 1px solid var(--border); }
.rules .sum { cursor: pointer; font-family: ui-monospace, monospace; font-size: 11px; word-break: break-all; }
.rules .ops { text-align: end; white-space: nowrap; }
button.mini { padding: 2px 7px; font-size: 12px; margin-left: 2px; }
.btns { display: flex; align-items: center; gap: 8px; }
.spacer { flex: 1; }
.err { color: #ef4444; font-size: 12px; }
.ok { color: #22c55e; font-size: 12px; }
.grid { display: grid; grid-template-columns: 130px 1fr; gap: 8px 10px; align-items: center; }
.grid label { font-size: 12px; opacity: 0.85; }
.sets { display: flex; flex-direction: column; gap: 4px; }
.check { display: inline-flex; align-items: center; gap: 6px; cursor: pointer; font-size: 13px; }
</style>
