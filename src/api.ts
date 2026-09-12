import { invoke } from "@tauri-apps/api/core";

/** Thin typed bridge to the Rust orchestrator commands (src-tauri/src/lib.rs). */

export function ping(): Promise<string> {
  return invoke<string>("ping");
}

export function coreVersion(): Promise<string> {
  return invoke<string>("core_version");
}

// ---- parse / import -----------------------------------------------------

export interface ParseError {
  line: number;
  snippet: string;
  reason: string;
}

export interface NodeMeta {
  id: string;
  remark: string;
  type: string;
  /** Full sing-box outbound options JSON. */
  out: unknown;
}

export interface ImportResult {
  nodes: NodeMeta[];
  errors: ParseError[];
}

export function parseText(text: string): Promise<ImportResult> {
  return invoke<ImportResult>("parse_text", { text });
}

export interface SubUserInfo {
  upload: number;
  download: number;
  total: number;
  expire?: number;
}

export interface SubscribeResult extends ImportResult {
  url: string;
  content_type?: string;
  userinfo?: SubUserInfo;
  group_id?: number;
}

export function subscribe(
  url: string,
  headers: Record<string, string> = {},
  saveName?: string,
): Promise<SubscribeResult> {
  return invoke<SubscribeResult>("subscribe", { url, headers, saveName });
}

// ---- storage ------------------------------------------------------------

export interface Group {
  id: number;
  name: string;
  sub_url?: string;
  user_agent?: string;
  extra_headers?: string;
  sub_userinfo?: string;
  updated_at?: string;
  last_update_epoch?: number;
  /** normal | strategy (auto urltest) | strategy-manual */
  kind?: string;
  /** member group ids (strategy groups) */
  members?: string;
}

export function subscriptionEdit(
  groupId: number,
  name: string,
  url: string,
  userAgent: string,
  extraHeadersJson: string,
): Promise<Group> {
  return invoke<Group>("subscription_edit", {
    groupId,
    name,
    url,
    userAgent,
    extraHeadersJson,
  });
}

/** Re-fetch with the subscription's saved URL + headers; replaces nodes. */
export function subscriptionRefresh(groupId: number): Promise<SubscribeResult> {
  return invoke<SubscribeResult>("subscription_refresh", { groupId });
}

export interface Node {
  id: string;
  group_id: number;
  type: string;
  remark: string;
  out: string;
}

export function groupsList(): Promise<Group[]> {
  return invoke<Group[]>("groups_list");
}

export function createGroup(name: string, subUrl?: string): Promise<Group> {
  return invoke<Group>("create_group", { name, subUrl });
}

/** v2rayN-style strategy group as a pseudo-node in the host group. */
export function createStrategyGroup(
  hostGroupId: number,
  name: string,
  auto: boolean,
  memberGroupIds: number[],
): Promise<Group> {
  return invoke<Group>("create_strategy_group", { hostGroupId, name, auto, memberGroupIds });
}

export function deleteGroup(groupId: number): Promise<void> {
  return invoke<void>("delete_group", { groupId });
}

export function renameGroup(groupId: number, name: string): Promise<void> {
  return invoke<void>("rename_group", { groupId, name });
}

export function nodesList(groupId: number): Promise<Node[]> {
  return invoke<Node[]>("nodes_list", { groupId });
}

export function deleteNode(groupId: number, nodeId: string): Promise<void> {
  return invoke<void>("delete_node", { groupId, nodeId });
}

/** Parse text and persist nodes into the group; returns parse result. */
export function importToGroup(groupId: number, text: string): Promise<ImportResult> {
  return invoke<ImportResult>("import_to_group", { groupId, text });
}

// ---- route profiles (rule-mode custom routing) ---------------------------

export interface RouteProfile {
  id: number;
  name: string;
  /** Final (default) outbound: proxy | direct | block. */
  final_out: string;
  /** Ordered JSON array of sing-box route rule objects. */
  rules_json: string;
  updated_at?: string;
}

export function routeProfilesList(): Promise<RouteProfile[]> {
  return invoke<RouteProfile[]>("route_profiles_list");
}

export function routeProfileCreate(
  name: string,
  finalOut: string,
  rulesJson: string,
): Promise<RouteProfile> {
  return invoke<RouteProfile>("route_profile_create", { name, finalOut, rulesJson });
}

export function routeProfileUpdate(
  profileId: number,
  name: string,
  finalOut: string,
  rulesJson: string,
): Promise<RouteProfile> {
  return invoke<RouteProfile>("route_profile_update", { profileId, name, finalOut, rulesJson });
}

export function routeProfileDelete(profileId: number): Promise<void> {
  return invoke<void>("route_profile_delete", { profileId });
}

/** Activate a custom profile; pass null to fall back to the built-in profile. */
export function routeProfileSetActive(profileId: number | null): Promise<void> {
  return invoke<void>("route_profile_set_active", { profileId });
}

// ---- settings / selection -----------------------------------------------

export interface Settings {
  current_group_id: number;
  port: number;
  mode: string; // global | direct | rule
  proxy_enabled: boolean;
  close_to_tray: boolean;
  log_level?: string; // debug | info | warn | error (legacy rows omit it)
  sort_by_delay?: boolean;
  filter_ipv6?: boolean;
  auto_update_subscriptions?: boolean;
  auto_update_minutes?: number;
  language?: string; // zh | en
  /** Launch at login (XDG autostart). */
  auto_start?: boolean;
  selected_by_group: Record<number, string>;
  /** Active custom routing profile id; null/absent => built-in bypass-mainland. */
  route_profile_id?: number | null;
}

export interface SettingsPatch {
  current_group_id?: number;
  port?: number;
  mode?: string;
  proxy_enabled?: boolean;
  close_to_tray?: boolean;
  log_level?: string;
  sort_by_delay?: boolean;
  filter_ipv6?: boolean;
  auto_update_subscriptions?: boolean;
  auto_update_minutes?: number;
  language?: string;
  auto_start?: boolean;
}

export function settingsGet(): Promise<Settings> {
  return invoke<Settings>("settings_get");
}

export function settingsSet(patch: SettingsPatch): Promise<Settings> {
  return invoke<Settings>("settings_set", { patch });
}

export function setNodeCurrent(groupId: number, nodeId: string): Promise<void> {
  return invoke<void>("set_node_current", { groupId, nodeId });
}

// ---- latency / core lifecycle -------------------------------------------

export interface MeasureView {
  delay_ms?: number | null;
  error?: string | null;
}

export function measureNode(groupId: number, nodeId: string): Promise<MeasureView> {
  return invoke<MeasureView>("measure_node", { groupId, nodeId });
}

export interface LatencyRow {
  node_id: string;
  delay_ms?: number | null;
  error?: string | null;
  tested_at: string;
}

/** Persisted latency results for a group (survive redraws/restarts). */
export function latencyList(groupId: number): Promise<LatencyRow[]> {
  return invoke<LatencyRow[]>("latency_list", { groupId });
}

export interface BatchRow {
  node_id: string;
  delay_ms?: number | null;
  error?: string | null;
}

/** v2rayN-style: probe the whole group in one core instance, concurrently. */
export function measureBatch(groupId: number): Promise<BatchRow[]> {
  return invoke<BatchRow[]>("measure_batch", { groupId });
}

export interface CopyView {
  inserted: number;
  duplicated: number;
}

/** Copy selected nodes (by id) from a source group into another group. */
export function copyNodes(
  sourceGroupId: number,
  targetGroupId: number,
  nodeIds: string[],
): Promise<CopyView> {
  return invoke<CopyView>("copy_nodes", { sourceGroupId, targetGroupId, nodeIds });
}

/** Share link for one node (copy link / QR share). */
export function nodeEncode(groupId: number, nodeId: string): Promise<string> {
  return invoke<string>("node_encode", { groupId, nodeId });
}

/** QR PNG data URL for a node's share link (generated in core). */
export function nodeQr(groupId: number, nodeId: string): Promise<string> {
  return invoke<string>("node_qr", { groupId, nodeId });
}

export interface CoreStatusView {
  running: boolean;
  started_at?: string;
  proxy_enabled: boolean;
  /** Session group + node of the running instance (absent while stopped). */
  group_id?: number | null;
  node_id?: string | null;
}

export interface RunResult {
  status: CoreStatusView;
  selected_node?: string | null;
}

/** Start/rebuild the core; targetGroupId overrides the persisted current
 *  group (used by the All aggregate view to run a node's owning group). */
export function coreStart(targetGroupId?: number): Promise<RunResult> {
  return invoke<RunResult>(
    "core_start",
    targetGroupId == null ? {} : { targetGroupId },
  );
}

export function coreStop(): Promise<CoreStatusView> {
  return invoke<CoreStatusView>("core_stop");
}

/** Toggle system proxy independently of the core lifecycle. */
export function proxySet(enabled: boolean): Promise<CoreStatusView> {
  return invoke<CoreStatusView>("proxy_set", { enabled });
}

export function coreStatus(): Promise<CoreStatusView> {
  return invoke<CoreStatusView>("core_status");
}

// ---- logs ---------------------------------------------------------------

export interface LogEntry {
  level: string; // error | warn | info | debug | other
  line: string;
}

export function logTail(limit?: number): Promise<LogEntry[]> {
  return invoke<LogEntry[]>("log_tail", { limit });
}
