//! Orchestrator layer: SQLite storage, core child supervision, system
//! proxy, tray and all UI-facing commands (see ../src/api.ts for the
//! mirrored TS contracts).

mod autostart;
mod core;
mod db;
mod runtime;
mod subscribe;
mod sysproxy;
#[cfg(target_os = "linux")]
mod tray_linux;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use db::{Db, NewNode, Settings};
use runtime::LogEntry;
use serde::Serialize;
use subscribe::{fetch_subscribe, SubscribeOutcome};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, State};

use crate::core::{CoreCtl, ImportResult};

/// Built-in aggregate group id ("All"). Its node list, persisted latency
/// and batch measurements span every normal group instead of one group's
/// membership; per-row operations still resolve to each node's real group.
const ALL_GROUP_ID: i64 = 1;

// ---- shared state -------------------------------------------------------

#[derive(Clone)]
pub struct AppState {
    db: Arc<Mutex<Db>>,
    ctl: CoreCtl,
    proxy_on: Arc<Mutex<bool>>,
    quitting: Arc<AtomicBool>,
    updating: Arc<Mutex<std::collections::HashSet<i64>>>,
    measuring: Arc<AtomicBool>,
    data_dir: Arc<std::path::PathBuf>,
}

impl AppState {
    fn open(app: &tauri::AppHandle) -> Result<AppState, Box<dyn std::error::Error>> {
        let dir = app.path().app_data_dir()?;
        std::fs::create_dir_all(&dir)?;
        let db = Db::open(&dir.join("nekos.db"))?;
        Ok(AppState {
            db: Arc::new(Mutex::new(db)),
            ctl: CoreCtl::new(),
            proxy_on: Arc::new(Mutex::new(false)),
            quitting: Arc::new(AtomicBool::new(false)),
            updating: Arc::new(Mutex::new(std::collections::HashSet::new())),
            measuring: Arc::new(AtomicBool::new(false)),
            data_dir: Arc::new(dir),
        })
    }

    /// Ensure the CN geoip/geosite rule-set files exist locally for rule
    /// mode; downloads (with mirror fallback) and caches them under the
    /// app data dir. Returns (geoip path, geosite path).
    async fn ensure_rule_assets(&self) -> Result<(String, String), String> {
        let dir = self.data_dir.join("rulesets");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let files = [("cn-ip.srs", "geoip/cn.srs"), ("cn-site.srs", "geosite/cn.srs")];
        let mut paths = Vec::new();
        for (name, rel) in files {
            let dest = dir.join(name);
            let cached = dest.is_file()
                && dest.metadata().map(|m| m.len() > 1000).unwrap_or(false);
            if !cached {
                let mirrors = [
                    format!("https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/sing/geo/{rel}"),
                    format!("https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@sing/geo/{rel}"),
                ];
                let mut last_err = "no mirror reachable".to_string();
                let mut saved = false;
                for url in mirrors {
                    match self.ctl.client().get(&url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            match resp.bytes().await {
                                Ok(bytes) if bytes.len() > 1000 => {
                                    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
                                    saved = true;
                                    break;
                                }
                                Ok(_) => last_err = format!("{url}: body too small"),
                                Err(e) => last_err = format!("{url}: {e}"),
                            }
                        }
                        Ok(resp) => last_err = format!("{url}: HTTP {}", resp.status()),
                        Err(e) => last_err = format!("{url}: {e}"),
                    }
                }
                if !saved {
                    return Err(format!(
                        "下载规则集 {name} 失败: {last_err}（启用「规则」模式需要网络）"
                    ));
                }
            }
            paths.push(dest.to_string_lossy().into_owned());
        }
        Ok((paths[0].clone(), paths[1].clone()))
    }

    fn settings(&self) -> Settings {
        self.db
            .lock()
            .map(|db| db.load_settings())
            .unwrap_or_default()
    }

    /// Assemble the `core run` session JSON from DB + settings.
    /// rule_assets must be Some for rule mode (caller ensures downloads).
    /// filter_v6 excludes IPv6-literal nodes from the session entirely.
    fn build_session(
        &self,
        group_id: i64,
        rule_assets: Option<(String, String)>,
        filter_v6: bool,
    ) -> Result<(String, String), String> {
        let settings = self.settings();
        let db = self.db.lock().map_err(|_| "db lock poisoned".to_string())?;
        // A selected "strat:<gid>" pseudo-node runs the member union
        // instead of the host group's own nodes.
        let strat_override: Option<i64> = settings
            .selected_by_group
            .get(&group_id)
            .and_then(|s| s.strip_prefix("strat:"))
            .and_then(|v| v.parse().ok());
        let source_group = strat_override.unwrap_or(group_id);
        let nodes = db.nodes_for_group(source_group).map_err(|e| format!("db: {e}"))?;
        if nodes.is_empty() {
            return Err("当前分组没有节点".into());
        }
        let nodes: Vec<&db::Node> = nodes
            .iter()
            .filter(|n| {
                if !filter_v6 {
                    return true;
                }
                let out: serde_json::Value = match serde_json::from_str(&n.out) {
                    Ok(v) => v,
                    Err(_) => return true, // keep unparsable rather than drop silently
                };
                !is_ipv6_server(&out)
            })
            .collect();
        if nodes.is_empty() {
            return Err("开启「过滤 IPv6 节点」后该分组没有可用节点".into());
        }
        let selected = settings
            .selected_by_group
            .get(&if strat_override.is_some() { source_group } else { group_id })
            .and_then(|id| nodes.iter().find(|n| &n.id == id))
            .map(|n| n.id.clone())
            .unwrap_or_else(|| nodes[0].id.clone());
        let entries: Vec<serde_json::Value> = nodes
            .iter()
            .filter_map(|n| -> Option<serde_json::Value> {
                let out: serde_json::Value = serde_json::from_str(&n.out).ok()?;
                Some(serde_json::json!({ "id": n.id, "out": out }))
            })
            .collect();
        let strat_kind: Option<String> = strat_override
            .and_then(|sg| db.group(sg).ok().flatten().map(|g| g.kind));
        let is_auto_strategy = strat_kind.as_deref() == Some("strategy");
        let mode = if is_auto_strategy {
            "strategy"
        } else {
            match settings.mode.as_str() {
                "direct" => "direct",
                "rule" => "rule",
                _ => "global",
            }
        };
        let mut session = serde_json::json!({
            "mode": mode,
            "inbound": { "listen": "127.0.0.1", "port": settings.port, "type": "mixed" },
            "entries": entries,
            "selected": selected,
            "log_level": settings.log_level,
        });
        if is_auto_strategy {
            // lowest-latency with automatic failover: the core's urltest
            // group re-probes on its interval and re-pins the fastest
            // healthy member when the current one fails.
            session["strategy"] = serde_json::json!({
                "url": "http://www.gstatic.com/generate_204",
                "interval": "1m",
                "tolerance": 50,
            });
            session["selected"] = serde_json::json!("auto");
        } else if mode == "rule" {
            let (ip, site) = rule_assets
                .ok_or_else(|| "规则模式需要 CN 规则集，请重试以触发下载".to_string())?;
            session["rule_assets"] = serde_json::json!({
                "geoip_cn": ip,
                "geosite_cn": site,
            });
            // A custom routing profile (rules + final outbound) overrides the
            // built-in bypass-mainland rules. A stale/deleted profile id
            // silently falls back to the built-in profile.
            if let Some(pid) = settings.route_profile_id {
                if let Some(p) = db.get_route_profile(pid).map_err(|e| format!("db: {e}"))? {
                    let rules: serde_json::Value = serde_json::from_str(&p.rules_json)
                        .map_err(|_| "路由档案规则数据损坏".to_string())?;
                    session["route"] = serde_json::json!({
                        "final": p.final_out,
                        "rules": rules,
                    });
                }
            }
        }
        let selected_id = session["selected"].as_str().unwrap_or_default().to_string();
        Ok((
            serde_json::to_string(&session).map_err(|e| e.to_string())?,
            selected_id,
        ))
    }
}

// ---- serializable view structs ------------------------------------------

#[derive(Serialize)]
pub struct CoreStatusView {
    pub running: bool,
    pub started_at: Option<String>,
    pub proxy_enabled: bool,
}

#[derive(Serialize)]
pub struct RunResult {
    pub status: CoreStatusView,
    pub selected_node: Option<String>,
}

#[derive(Serialize)]
pub struct MeasureView {
    pub delay_ms: Option<i64>,
    pub error: Option<String>,
}

// ---- helpers ------------------------------------------------------------

fn with_db<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Db) -> Result<T, String>,
) -> Result<T, String> {
    let db = state
        .db
        .lock()
        .map_err(|_| "db lock poisoned".to_string())?;
    f(&db)
}

/// True when a node's server field is an IPv6 literal (domains pass).
/// Tolerates "[...]" brackets and "%zone" suffixes.
fn is_ipv6_server(out: &serde_json::Value) -> bool {
    let host = match out.get("server").and_then(|s| s.as_str()) {
        Some(h) => h.trim(),
        None => return false,
    };
    let host = host.strip_prefix('[').and_then(|h| h.split_once(']')).map(|(h, _)| h).unwrap_or(host);
    let host = host.split_once('%').map(|(h, _)| h).unwrap_or(host);
    host.parse::<std::net::IpAddr>().map(|ip| ip.is_ipv6()).unwrap_or(false)
}

/// Apply the "filter IPv6 nodes" preference to parsed nodes.
fn filter_nodes(nodes: &[core::NodeMeta], filter_ipv6: bool) -> Vec<core::NodeMeta> {
    if !filter_ipv6 {
        return nodes.to_vec();
    }
    nodes.iter().filter(|n| !is_ipv6_server(&n.out)).cloned().collect()
}

fn shutdown_all(state: &AppState) {
    // Kill the control process (the instance dies with it) and restore the
    // system proxy.
    state.ctl.shutdown();
    let proxy = state.proxy_on.lock().map(|g| *g).unwrap_or(false);
    if proxy {
        let _ = sysproxy::disable();
        if let Ok(mut p) = state.proxy_on.lock() {
            *p = false;
        }
    }
}

// ---- basic commands ------------------------------------------------------

#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

#[tauri::command]
fn core_version(state: State<'_, AppState>) -> Result<String, String> {
    state.ctl.version()
}

#[tauri::command]
async fn parse_text(state: State<'_, AppState>, text: String) -> Result<ImportResult, String> {
    let ctl = state.ctl.clone();
    tauri::async_runtime::spawn_blocking(move || ctl.parse(&text))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn groups_list(state: State<'_, AppState>) -> Result<Vec<db::Group>, String> {
    with_db(&state, |db| db.list_groups().map_err(|e| e.to_string()))
}

#[tauri::command]
async fn nodes_list(state: State<'_, AppState>, group_id: i64) -> Result<Vec<db::Node>, String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        // "All" aggregates the real nodes of every normal group; strategy
        // pseudo-rows keep appearing only in their host group's own list.
        if group_id == ALL_GROUP_ID {
            return db.nodes_all().map_err(|e| e.to_string());
        }
        let mut nodes = db.nodes_for_group(group_id).map_err(|e| e.to_string())?;
        // strategy (policy) groups masquerade as nodes in their host group
        for s in db.strategies_for(group_id).map_err(|e| e.to_string())? {
            let auto = if s.kind == "strategy" { "⚡" } else { "◈" };
            nodes.push(db::Node {
                id: format!("strat:{}", s.id),
                group_id: s.id,
                r#type: "strategy".into(),
                remark: format!("{} {auto}", s.name),
                out: "{}".into(),
            });
        }
        Ok(nodes)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Parse text and persist every resulting node into the group.
/// Returns the parse result so the UI can report counts.
#[tauri::command]
async fn import_to_group(
    state: State<'_, AppState>,
    group_id: i64,
    text: String,
) -> Result<ImportResult, String> {
    let ctl = state.ctl.clone();
    let parsed = tauri::async_runtime::spawn_blocking(move || ctl.parse(&text))
        .await
        .map_err(|e| e.to_string())??;
    let kept = filter_nodes(&parsed.nodes, state.settings().filter_ipv6);
    if kept.is_empty() && !parsed.nodes.is_empty() {
        return Err("「过滤 IPv6 节点」已开启：本次内容没有可用节点，未导入".into());
    }
    let new_nodes: Vec<NewNode> = kept
        .iter()
        .map(|n| NewNode {
            id: n.id.clone(),
            r#type: n.r#type.clone(),
            remark: n.remark.clone(),
            out: n.out.to_string(),
        })
        .collect();
    if !new_nodes.is_empty() {
        let db = state.db.clone();
        tauri::async_runtime::spawn_blocking(move || {
            db.lock()
                .map_err(|_| "db lock poisoned".to_string())?
                .upsert_nodes(group_id, &new_nodes)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    Ok(ImportResult { nodes: kept, errors: parsed.errors })
}

/// Create a strategy group (v2rayN policy group) that shows up as a
/// pseudo-node inside the host group's node list.
/// auto=true measures members on start and pins the fastest node.
#[tauri::command]
async fn create_strategy_group(
    state: State<'_, AppState>,
    host_group_id: i64,
    name: String,
    auto: bool,
    member_group_ids: Vec<i64>,
) -> Result<db::Group, String> {
    if name.trim().is_empty() {
        return Err("名称不能为空".into());
    }
    if member_group_ids.is_empty() {
        return Err("至少选择一个成员分组".into());
    }
    let kind = if auto { "strategy" } else { "strategy-manual" }.to_string();
    let members_json =
        serde_json::to_string(&member_group_ids).map_err(|e| e.to_string())?;
    let db = state.db.clone();
    let gid = tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        db.create_strategy(host_group_id, name.trim(), &kind, &members_json)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    with_db(&state, |db| {
        db.group(gid)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "分组消失".to_string())
    })
}

#[tauri::command]
async fn create_group(
    state: State<'_, AppState>,
    name: String,
    sub_url: Option<String>,
) -> Result<db::Group, String> {
    let db = state.db.clone();
    let gid = tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let id = db
            .create_group(&name, sub_url.as_deref())
            .map_err(|e| e.to_string())?;
        db.group(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "group vanished".to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(gid)
}

#[tauri::command]
async fn rename_group(
    state: State<'_, AppState>,
    group_id: i64,
    name: String,
) -> Result<(), String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        db.lock()
            .map_err(|_| "db lock poisoned".to_string())?
            .rename_group(group_id, &name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(())
}

#[tauri::command]
async fn delete_group(state: State<'_, AppState>, group_id: i64) -> Result<(), String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        db.lock()
            .map_err(|_| "db lock poisoned".to_string())?
            .delete_group(group_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(())
}

#[tauri::command]
async fn delete_node(
    state: State<'_, AppState>,
    group_id: i64,
    node_id: String,
) -> Result<(), String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        db.lock()
            .map_err(|_| "db lock poisoned".to_string())?
            .delete_node(group_id, &node_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(())
}

// ---- route profiles (rule-mode custom routing) ---------------------------

fn route_final_valid(v: &str) -> bool {
    matches!(v, "proxy" | "direct" | "block")
}

fn route_rules_valid(s: &str) -> bool {
    serde_json::from_str::<Vec<serde_json::Value>>(s)
        .map(|rules| rules.iter().all(|r| r.is_object()))
        .unwrap_or(false)
}

#[tauri::command]
fn route_profiles_list(state: State<'_, AppState>) -> Result<Vec<db::RouteProfile>, String> {
    with_db(&state, |db| db.list_route_profiles().map_err(|e| e.to_string()))
}

#[tauri::command]
async fn route_profile_create(
    state: State<'_, AppState>,
    name: String,
    final_out: String,
    rules_json: String,
) -> Result<db::RouteProfile, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("请输入路由档案名称".into());
    }
    if !route_final_valid(&final_out) {
        return Err("兜底出口必须是 proxy / direct / block".into());
    }
    if !route_rules_valid(&rules_json) {
        return Err("规则内容必须是 sing-box 路由规则对象数组".into());
    }
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<db::RouteProfile, String> {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let id = db
            .create_route_profile(&name, &final_out, &rules_json)
            .map_err(|e| e.to_string())?;
        db.get_route_profile(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "路由档案不存在".into())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn route_profile_update(
    state: State<'_, AppState>,
    profile_id: i64,
    name: String,
    final_out: String,
    rules_json: String,
) -> Result<db::RouteProfile, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("请输入路由档案名称".into());
    }
    if !route_final_valid(&final_out) {
        return Err("兜底出口必须是 proxy / direct / block".into());
    }
    if !route_rules_valid(&rules_json) {
        return Err("规则内容必须是 sing-box 路由规则对象数组".into());
    }
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<db::RouteProfile, String> {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        db.update_route_profile(profile_id, &name, &final_out, &rules_json)
            .map_err(|e| e.to_string())?;
        db.get_route_profile(profile_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "路由档案不存在".into())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn route_profile_delete(state: State<'_, AppState>, profile_id: i64) -> Result<(), String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        db.delete_route_profile(profile_id).map_err(|e| e.to_string())?;
        let mut s = db.load_settings();
        if s.route_profile_id == Some(profile_id) {
            // deleting the active profile falls back to the built-in one
            s.route_profile_id = None;
            db.save_settings(&s).map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Activate a custom routing profile (Some(id)) or fall back to the built-in
/// bypass-mainland profile (None).
#[tauri::command]
async fn route_profile_set_active(
    state: State<'_, AppState>,
    profile_id: Option<i64>,
) -> Result<(), String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        if let Some(id) = profile_id {
            if db.get_route_profile(id).map_err(|e| e.to_string())?.is_none() {
                return Err("路由档案不存在".into());
            }
        }
        let mut s = db.load_settings();
        s.route_profile_id = profile_id;
        db.save_settings(&s).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn settings_get(state: State<'_, AppState>) -> Settings {
    state.settings()
}

#[derive(serde::Deserialize)]
struct SettingsPatch {
    current_group_id: Option<i64>,
    port: Option<u16>,
    mode: Option<String>,
    proxy_enabled: Option<bool>,
    close_to_tray: Option<bool>,
    log_level: Option<String>,
    sort_by_delay: Option<bool>,
    filter_ipv6: Option<bool>,
    auto_update_subscriptions: Option<bool>,
    auto_update_minutes: Option<u32>,
    language: Option<String>,
    auto_start: Option<bool>,
}

#[tauri::command]
async fn settings_set(
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> Result<Settings, String> {
    let prev = state.settings();
    let db = state.db.clone();
    let saved = tauri::async_runtime::spawn_blocking(move || -> Result<Settings, String> {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let mut s = db.load_settings();
        if let Some(v) = patch.current_group_id {
            s.current_group_id = v;
        }
        if let Some(v) = patch.port {
            s.port = v;
        }
        if let Some(v) = patch.mode {
            s.mode = v;
        }
        if let Some(v) = patch.proxy_enabled {
            s.proxy_enabled = v;
        }
        if let Some(v) = patch.close_to_tray {
            s.close_to_tray = v;
        }
        if let Some(v) = patch.log_level {
            s.log_level = v;
        }
        if let Some(v) = patch.sort_by_delay {
            s.sort_by_delay = v;
        }
        if let Some(v) = patch.filter_ipv6 {
            s.filter_ipv6 = v;
        }
        if let Some(v) = patch.auto_update_subscriptions {
            s.auto_update_subscriptions = v;
        }
        if let Some(v) = patch.auto_update_minutes {
            s.auto_update_minutes = v.clamp(5, 60 * 24 * 7);
        }
        if let Some(v) = patch.language {
            s.language = v;
        }
        if let Some(v) = patch.auto_start {
            s.auto_start = v;
        }
        db.save_settings(&s).map_err(|e| e.to_string())?;
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())??;

    // Apply the side effect that needs the running app: the XDG autostart
    // entry.
    if saved.auto_start != prev.auto_start {
        let res = if saved.auto_start {
            autostart::enable()
        } else {
            autostart::disable()
        };
        if let Err(e) = res {
            return Err(format!("开机自启: {e}"));
        }
    }
    Ok(saved)
}

/// Last captured core log lines (newest last).
#[tauri::command]
fn log_tail(state: State<'_, AppState>, limit: Option<usize>) -> Vec<LogEntry> {
    state.ctl.tail_logs(limit.unwrap_or(300))
}

/// Remember which node is "current" for a group.
#[tauri::command]
async fn set_node_current(
    state: State<'_, AppState>,
    group_id: i64,
    node_id: String,
) -> Result<(), String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        if !node_id.starts_with("strat:") {
            if db
                .node(group_id, &node_id)
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Err("节点不存在".to_string());
            }
        }
        let mut s = db.load_settings();
        s.selected_by_group.insert(group_id, node_id);
        db.save_settings(&s).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One measured node row (v2rayN-style batch outcome).
#[derive(Serialize)]
pub struct BatchRow {
    pub node_id: String,
    pub delay_ms: Option<i64>,
    pub error: Option<String>,
}

/// (id, outbound JSON, owning group id) — for strategy groups the owning
/// group is where latency results are stored.
type EntryOwned = (String, serde_json::Value, i64);

/// Resolve a group's node entries: plain list for normal groups, the
/// de-duplicated union of member groups for strategy groups.
fn resolve_entries(
    db: &Mutex<Db>,
    group_id: i64,
    ids: Option<&[String]>,
    filter_v6: bool,
) -> Result<Vec<EntryOwned>, String> {
    let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
    let nodes = if group_id == ALL_GROUP_ID {
        db.nodes_all().map_err(|e| format!("db: {e}"))?
    } else {
        db.nodes_for_group(group_id).map_err(|e| format!("db: {e}"))?
    };
    let entries: Vec<EntryOwned> = nodes
        .iter()
        .filter(|n| ids.map(|ids| ids.contains(&n.id)).unwrap_or(true))
        .filter_map(|n| {
            let out: serde_json::Value = serde_json::from_str(&n.out).ok()?;
            if filter_v6 && is_ipv6_server(&out) {
                return None;
            }
            Some((n.id.clone(), out, n.group_id))
        })
        .collect();
    if entries.is_empty() {
        return Err("没有可测速的节点".into());
    }
    Ok(entries)
}

fn entries_session(entries: &[EntryOwned]) -> String {
    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|(id, out, _)| serde_json::json!({ "id": id, "out": out }))
        .collect();
    serde_json::json!({ "entries": items, "timeout_s": 5 }).to_string()
}

/// Probe entries in one core instance; persist results under each node's
/// owning group.
fn probe_and_persist(
    db: &Mutex<Db>,
    ctl: &CoreCtl,
    entries: &[EntryOwned],
) -> Result<Vec<core::UrlTestRow>, String> {
    let rows = ctl.urltest(&entries_session(entries))?;
    let owner: std::collections::HashMap<&str, i64> =
        entries.iter().map(|(id, _, g)| (id.as_str(), *g)).collect();
    if let Ok(db) = db.lock() {
        for r in &rows {
            if let Some(g) = owner.get(r.id.as_str()) {
                let _ = db.upsert_latency(*g, &r.id, r.delay_ms, r.error.as_deref(), &unix_now_secs());
            }
        }
    }
    Ok(rows)
}

/// Measure one node (single-entry batch, same engine as batch testing).
/// A "strat:<gid>" pseudo-node measures ALL its member nodes (like batch
/// testing the strategy) and reports the fastest member's delay.
#[tauri::command]
async fn measure_node(
    state: State<'_, AppState>,
    group_id: i64,
    node_id: String,
) -> Result<MeasureView, String> {
    let _guard = MeasureGuard::acquire(&state.measuring)?;
    if let Some(sgid) = node_id
        .strip_prefix("strat:")
        .and_then(|v| v.parse::<i64>().ok())
    {
        let db = state.db.clone();
        let ctl = state.ctl.clone();
        return tauri::async_runtime::spawn_blocking(move || -> Result<MeasureView, String> {
            let entries = resolve_entries(&db, sgid, None, false)?;
            let rows = probe_and_persist(&db, &ctl, &entries)?;
            let failed = rows.iter().filter(|r| r.error.is_some()).count();
            match rows.iter().filter_map(|r| r.delay_ms).min() {
                Some(ms) => Ok(MeasureView {
                    delay_ms: Some(ms),
                    error: None,
                }),
                None => Ok(MeasureView {
                    delay_ms: None,
                    error: Some(format!("成员全部失败（{failed}/{}）", rows.len())),
                }),
            }
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    let db = state.db.clone();
    let ctl = state.ctl.clone();
    let ids = vec![node_id.clone()];
    let row = tauri::async_runtime::spawn_blocking(move || -> Result<core::UrlTestRow, String> {
        let entries = resolve_entries(&db, group_id, Some(&ids), false)?;
        let rows = probe_and_persist(&db, &ctl, &entries)?;
        Ok(rows.into_iter().next().unwrap_or(core::UrlTestRow {
            id: node_id,
            delay_ms: None,
            error: Some("没有返回结果".into()),
        }))
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(MeasureView { delay_ms: row.delay_ms, error: row.error })
}

/// Live per-node latency event pushed to the UI as each node finishes
/// (frontend listens on "latency:row").
#[derive(Serialize, Clone)]
struct LatencyRowEvent {
    node_id: String,
    delay_ms: Option<i64>,
    error: Option<String>,
}

/// Random hex id for a tracked batch-probe run.
fn run_id_hex() -> String {
    let mut buf = [0u8; 16];
    let _ = getrandom::getrandom(&mut buf);
    let mut s = String::with_capacity(32);
    for b in buf {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Measure every node of a group in one instance, persisted per node.
/// Rows are pushed to the UI one by one as the core finishes each node
/// ("latency:row" events) instead of arriving only when the whole batch
/// completes; the command returns the full result array at the end.
#[tauri::command]
async fn measure_batch(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    group_id: i64,
) -> Result<Vec<BatchRow>, String> {
    let _guard = MeasureGuard::acquire(&state.measuring)?;
    let db = state.db.clone();
    let ctl = state.ctl.clone();
    let filter_v6 = state.settings().filter_ipv6;

    let db_r = db.clone();
    let entries = tauri::async_runtime::spawn_blocking(move || {
        resolve_entries(&db_r, group_id, None, filter_v6)
    })
    .await
    .map_err(|e| e.to_string())??;

    let run_id = run_id_hex();
    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|(id, out, _)| serde_json::json!({ "id": id, "out": out }))
        .collect();
    let payload = serde_json::json!({
        "entries": items,
        "timeout_s": 5,
        "run_id": run_id.as_str(),
    })
    .to_string();

    // Kick the batch on a worker thread; results stream back through
    // core.url_test_progress polls. Errors land in the shared slot.
    let err_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let ctl_worker = ctl.clone();
    let err_worker = err_slot.clone();
    std::thread::spawn(move || {
        if let Err(e) = ctl_worker.urltest(&payload) {
            *err_worker.lock().unwrap() = Some(e);
        }
    });

    // Emit each node the first time its value appears; re-emit when a
    // retry round replaces an error with a delay.
    let mut values: HashMap<String, (Option<i64>, Option<String>)> = HashMap::new();
    let mut accumulated: Vec<core::UrlTestRow> = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    let done_rows: Arc<Mutex<Option<Vec<core::UrlTestRow>>>> = Arc::new(Mutex::new(None));
    let done_slot = done_rows.clone();
    loop {
        if let Some(e) = err_slot.lock().unwrap().clone() {
            return Err(e);
        }
        let ctl_poll = ctl.clone();
        let rid = run_id.clone();
        let prog = tauri::async_runtime::spawn_blocking(move || {
            ctl_poll.url_test_progress(&rid)
        })
        .await
        .map_err(|e| e.to_string())??;
        if prog.found {
            for r in &prog.results {
                let key = (r.delay_ms, r.error.clone());
                if values.get(&r.id) != Some(&key) {
                    values.insert(r.id.clone(), key);
                    let _ = app.emit(
                        "latency:row",
                        LatencyRowEvent {
                            node_id: r.id.clone(),
                            delay_ms: r.delay_ms,
                            error: r.error.clone(),
                        },
                    );
                }
            }
            if prog.done {
                *done_slot.lock().unwrap() = Some(prog.results);
                break;
            }
            accumulated = prog.results;
        }
        if std::time::Instant::now() > deadline {
            break; // run lost (daemon respawned mid-batch): report what arrived
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    let rows = done_rows
        .lock()
        .unwrap()
        .clone()
        .unwrap_or(accumulated);
    // Persist under each node's owning group (mirrors probe_and_persist).
    {
        let db = db.clone();
        let owner: HashMap<String, i64> = entries
            .iter()
            .map(|(id, _, g)| (id.clone(), *g))
            .collect();
        let rows = rows.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Ok(db) = db.lock() {
                for r in &rows {
                    if let Some(g) = owner.get(&r.id) {
                        let _ = db.upsert_latency(
                            *g,
                            &r.id,
                            r.delay_ms,
                            r.error.as_deref(),
                            &unix_now_secs(),
                        );
                    }
                }
            }
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(rows
        .into_iter()
        .map(|r| BatchRow {
            node_id: r.id,
            delay_ms: r.delay_ms,
            error: r.error,
        })
        .collect())
}

/// Copy selected nodes from one group into another (snapshot semantics).
#[derive(Serialize)]
pub struct CopyView {
    pub inserted: usize,
    pub duplicated: usize,
}

#[tauri::command]
async fn copy_nodes(
    state: State<'_, AppState>,
    source_group_id: i64,
    target_group_id: i64,
    node_ids: Vec<String>,
) -> Result<CopyView, String> {
    if source_group_id == target_group_id {
        return Err("源与目标不能是同一分组".into());
    }
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let mut real = Vec::new();
        let mut strats = Vec::new();
        for id in &node_ids {
            if let Some(sid) = id.strip_prefix("strat:") {
                if let Ok(sid) = sid.parse::<i64>() {
                    strats.push(sid);
                    continue;
                }
            }
            real.push(id.clone());
        }
        let (mut inserted, mut duplicated) = if real.is_empty() {
            (0, 0)
        } else {
            db.copy_group_nodes(source_group_id, target_group_id, &real)
                .map_err(|e| e.to_string())?
        };
        for sid in strats {
            if db.add_strategy_host(sid, target_group_id).map_err(|e| e.to_string())? {
                inserted += 1;
            } else {
                duplicated += 1;
            }
        }
        Ok(CopyView { inserted, duplicated })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Share link for one node ("copy link" / QR share).
#[tauri::command]
async fn node_encode(
    state: State<'_, AppState>,
    group_id: i64,
    node_id: String,
) -> Result<String, String> {
    let db = state.db.clone();
    let ctl = state.ctl.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let node = db
            .node(group_id, &node_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "节点不存在".to_string())?;
        let out: serde_json::Value =
            serde_json::from_str(&node.out).map_err(|e| format!("node json: {e}"))?;
        ctl.encode(&node.remark, &out)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// QR data URL for one node's share link (server-generated PNG).
#[tauri::command]
async fn node_qr(
    state: State<'_, AppState>,
    group_id: i64,
    node_id: String,
) -> Result<String, String> {
    let db = state.db.clone();
    let ctl = state.ctl.clone();
    let b64 = tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let node = db
            .node(group_id, &node_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "节点不存在".to_string())?;
        let out: serde_json::Value =
            serde_json::from_str(&node.out).map_err(|e| format!("node json: {e}"))?;
        ctl.qr(&node.remark, &out, 300)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(format!("data:image/png;base64,{b64}"))
}

/// Persisted latency results for a group (strategy groups aggregate their
/// member groups' rows). Shown until re-tested.
#[tauri::command]
async fn latency_list(
    state: State<'_, AppState>,
    group_id: i64,
) -> Result<Vec<db::LatencyRow>, String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        // For "All" collect every group's persisted rows; otherwise the
        // owners of the group's own nodes (strategy groups aggregate their
        // member groups' rows).
        let mut owners: Vec<i64> = if group_id == ALL_GROUP_ID {
            db.list_groups()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|g| g.id)
                .collect()
        } else {
            let nodes = db.nodes_for_group(group_id).map_err(|e| e.to_string())?;
            let mut owners: Vec<i64> = nodes.iter().map(|n| n.group_id).collect();
            owners.sort_unstable();
            owners.dedup();
            owners
        };
        owners.sort_unstable();
        owners.dedup();
        let mut rows = Vec::new();
        for owner in owners {
            for row in db.list_latency(owner).map_err(|e| e.to_string())? {
                rows.push(row);
            }
        }
        Ok(rows)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- subscription management --------------------------------------------

/// Edit a subscription group's metadata (name, url, UA, extra headers).
/// Node content is untouched; call subscription_refresh to re-fetch.
#[tauri::command]
async fn subscription_edit(
    state: State<'_, AppState>,
    group_id: i64,
    name: String,
    url: String,
    user_agent: String,
    extra_headers_json: String,
) -> Result<db::Group, String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let current = db
            .group(group_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "分组不存在".to_string())?;
        if group_id != 1 && !name.trim().is_empty() {
            db.rename_group(group_id, name.trim())
                .map_err(|e| e.to_string())?;
        }
        let ua = if user_agent.trim().is_empty() {
            None
        } else {
            Some(user_agent.trim().to_string())
        };
        let extras = if extra_headers_json.trim().is_empty()
            || extra_headers_json.trim() == "{}"
        {
            None
        } else {
            Some(extra_headers_json.trim().to_string())
        };
        let url = if url.trim().is_empty() { None } else { Some(url.trim().to_string()) };
        db.update_group_submeta(
            group_id,
            url.as_deref(),
            ua.as_deref(),
            extras.as_deref(),
            current.sub_userinfo.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        db.group(group_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "分组消失".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// RAII guard preventing concurrent refreshes of the same group.
struct RefreshGuard {
    updating: Arc<Mutex<std::collections::HashSet<i64>>>,
    group_id: i64,
}

/// Single-flight guard for latency tests: overlapping batch/node measures
/// double the probe burst against the exit servers, which trips their
/// per-IP session limits and makes valid nodes time out. Second concurrent
/// measure is rejected instead of stacked.
struct MeasureGuard {
    flag: Arc<AtomicBool>,
}

impl MeasureGuard {
    fn acquire(flag: &Arc<AtomicBool>) -> Result<MeasureGuard, String> {
        if flag
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("已有测速任务进行中，请稍候再试".into());
        }
        Ok(MeasureGuard { flag: flag.clone() })
    }
}

impl Drop for MeasureGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

impl RefreshGuard {
    fn acquire(
        updating: &Arc<Mutex<std::collections::HashSet<i64>>>,
        group_id: i64,
    ) -> Result<RefreshGuard, String> {
        let mut set = updating.lock().map_err(|_| "updating lock poisoned".to_string())?;
        if !set.insert(group_id) {
            return Err("该订阅正在更新中".into());
        }
        Ok(RefreshGuard { updating: updating.clone(), group_id })
    }
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.updating.lock() {
            set.remove(&self.group_id);
        }
    }
}

/// Re-fetch a subscription group using its saved URL + headers and replace
/// the group's nodes. Shared by the manual refresh command and the
/// background auto-update loop.
async fn refresh_subscription_group(
    db: Arc<Mutex<Db>>,
    ctl: CoreCtl,
    updating: Arc<Mutex<std::collections::HashSet<i64>>>,
    filter_v6: bool,
    group_id: i64,
) -> Result<subscribe::SubscribeOutcome, String> {
    let _guard = RefreshGuard::acquire(&updating, group_id)?;
    let client = ctl.client().clone();
    let db_read = db.clone();
    let db_write = db.clone();

    let (url, headers) = tauri::async_runtime::spawn_blocking(move || -> Result<(String, HashMap<String, String>), String> {
        let db = db_read.lock().map_err(|_| "db lock poisoned".to_string())?;
        let g = db
            .group(group_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "分组不存在".to_string())?;
        let url = g.sub_url.ok_or_else(|| "该分组不是订阅组（没有订阅 URL）".to_string())?;
        let mut headers: HashMap<String, String> = HashMap::new();
        if let Some(ua) = g.user_agent.filter(|v| !v.is_empty()) {
            headers.insert("User-Agent".into(), ua);
        }
        if let Some(extras) = g.extra_headers {
            if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&extras) {
                headers.extend(map);
            }
        }
        Ok((url, headers))
    })
    .await
    .map_err(|e| e.to_string())??;

    let (body, content_type, userinfo) = fetch_subscribe(&client, &url, &headers).await?;
    let text = String::from_utf8_lossy(&body).into_owned();
    let parsed = tauri::async_runtime::spawn_blocking(move || ctl.parse(&text))
        .await
        .map_err(|e| format!("parse task failed: {e}"))??;

    if parsed.nodes.is_empty() {
        let detail = parsed
            .errors
            .first()
            .map(|e| e.reason.as_str())
            .unwrap_or("未知原因");
        return Err(format!(
            "抓取解析失败（0 节点 / {} 错误），现有节点未改动：{detail}",
            parsed.errors.len()
        ));
    }

    let kept = filter_nodes(&parsed.nodes, filter_v6);
    if kept.is_empty() && !parsed.nodes.is_empty() {
        return Err("「过滤 IPv6 节点」已开启：该订阅更新后没有可用节点，现有节点未改动".into());
    }

    let nodes: Vec<NewNode> = kept
        .iter()
        .map(|n| NewNode {
            id: n.id.clone(),
            r#type: n.r#type.clone(),
            remark: n.remark.clone(),
            out: n.out.to_string(),
        })
        .collect();
    let userinfo_json = userinfo
        .as_ref()
        .map(|u| serde_json::to_string(u).unwrap_or_default());
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let db = db_write.lock().map_err(|_| "db lock poisoned".to_string())?;
        db.replace_group_nodes(group_id, &nodes)
            .map_err(|e| e.to_string())?;
        let current = db.group(group_id).map_err(|e| e.to_string())?;
        db.update_group_submeta(
            group_id,
            current.as_ref().and_then(|g| g.sub_url.as_deref()),
            current.as_ref().and_then(|g| g.user_agent.as_deref()),
            current.as_ref().and_then(|g| g.extra_headers.as_deref()),
            userinfo_json.as_deref(),
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(subscribe::SubscribeOutcome {
        url,
        content_type,
        userinfo,
        group_id: Some(group_id),
        parsed: ImportResult { nodes: kept, errors: parsed.errors },
    })
}

/// Manual refresh (UI button).
#[tauri::command]
async fn subscription_refresh(
    state: State<'_, AppState>,
    group_id: i64,
) -> Result<subscribe::SubscribeOutcome, String> {
    refresh_subscription_group(
        state.db.clone(),
        state.ctl.clone(),
        state.updating.clone(),
        state.settings().filter_ipv6,
        group_id,
    )
    .await
}

fn unix_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn unix_now_secs() -> String {
    unix_now().to_string()
}

/// Background loop: every minute refresh subscriptions whose last successful
/// update is older than the configured interval (when auto-update is on).
async fn auto_update_loop(state: AppState) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let settings = state.settings();
        if !settings.auto_update_subscriptions {
            continue;
        }
        let interval_secs = u64::from(settings.auto_update_minutes.max(5)) * 60;
        let now = unix_now();
        let due: Vec<i64> = {
            let db = match state.db.lock() {
                Ok(db) => db,
                Err(_) => continue,
            };
            match db.list_groups() {
                Ok(groups) => groups
                    .into_iter()
                    .filter(|g| g.sub_url.is_some())
                    .filter(|g| {
                        let last = u64::try_from(g.last_update_epoch.unwrap_or(0)).unwrap_or(0);
                        now.saturating_sub(last) >= interval_secs
                    })
                    .map(|g| g.id)
                    .collect(),
                Err(_) => continue,
            }
        };
        for group_id in due {
            if let Err(e) = refresh_subscription_group(
                state.db.clone(),
                state.ctl.clone(),
                state.updating.clone(),
                settings.filter_ipv6,
                group_id,
            )
            .await
            {
                eprintln!("auto subscription refresh #{group_id}: {e}");
            }
        }
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;
    use crate::core::NodeMeta;

    fn meta(server: &str) -> NodeMeta {
        NodeMeta {
            id: "id".into(),
            remark: "r".into(),
            r#type: "anytls".into(),
            out: serde_json::json!({ "type": "anytls", "server": server }),
        }
    }

    #[test]
    fn ipv6_literal_detection() {
        assert!(is_ipv6_server(&meta("2001:db8::1").out));
        assert!(is_ipv6_server(&meta("[240e:1234::abcd]").out));
        assert!(is_ipv6_server(&meta("fe80::1%eth0").out));
        assert!(!is_ipv6_server(&meta("example.com").out));
        assert!(!is_ipv6_server(&meta("1.2.3.4").out));
    }

    #[test]
    fn filter_keeps_domains_and_v4() {
        let nodes = vec![
            meta("example.com"),
            meta("1.2.3.4"),
            meta("2001:db8::1"),
            meta("[240e::1]"),
        ];
        let kept = filter_nodes(&nodes, true);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|n| !is_ipv6_server(&n.out)));
        // toggle off keeps everything
        assert_eq!(filter_nodes(&nodes, false).len(), 4);
    }
}

// ---- core lifecycle -----------------------------------------------------

#[tauri::command]
fn core_status(state: State<'_, AppState>) -> CoreStatusView {
    core_status_raw(&state)
}

fn core_status_raw(state: &AppState) -> CoreStatusView {
    let (running, started_at) = state.ctl.mirror();
    let proxy_enabled = state.proxy_on.lock().map(|g| *g).unwrap_or(false);
    CoreStatusView {
        running,
        started_at,
        proxy_enabled,
    }
}

/// Start (or rebuild, when already running) the core with a group's session.
/// The session group defaults to the persisted current group; the "All"
/// aggregate view passes the owning group of the node being started so the
/// running proxy follows that node without leaving the All view.
#[tauri::command]
async fn core_start(
    state: State<'_, AppState>,
    target_group_id: Option<i64>,
) -> Result<RunResult, String> {
    let settings = state.settings();
    let group_id = target_group_id.unwrap_or(settings.current_group_id);
    let rule_assets = if settings.mode == "rule" {
        Some(state.ensure_rule_assets().await?)
    } else {
        None
    };
    let (session_json, selected) =
        state.build_session(group_id, rule_assets, settings.filter_ipv6)?;

    let ctl = state.ctl.clone();
    // System proxy is NOT auto-enabled on start: the user controls it
    // independently via the toolbar switch (proxy_set). core.start over
    // RPC boots the instance (or rebuilds it in-process when already
    // running — the node-switch path).
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        ctl.core_start(&session_json)
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(RunResult {
        status: core_status_raw(&state),
        selected_node: Some(selected),
    })
}

/// Toggle the system proxy independently of the core lifecycle.
/// Enabling requires the core to be listening; disabling restores the
/// system proxy immediately. The choice is persisted.
#[tauri::command]
async fn proxy_set(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<CoreStatusView, String> {
    let db = state.db.clone();
    let proxy_on = state.proxy_on.clone();
    let running = state.ctl.mirror().0;
    if enabled && !running {
        return Err("内核未运行，无法开启系统代理（请先「启动」）".into());
    }
    let port = state.settings().port;
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        if enabled {
            sysproxy::enable(port)?;
        } else {
            sysproxy::disable()?;
        }
        *proxy_on.lock().unwrap() = enabled;
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let mut s = db.load_settings();
        s.proxy_enabled = enabled;
        db.save_settings(&s).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(core_status_raw(&state))
}

#[tauri::command]
async fn core_stop(state: State<'_, AppState>) -> Result<CoreStatusView, String> {
    let ctl = state.ctl.clone();
    let proxy_on = state.proxy_on.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        ctl.core_stop()?;
        if proxy_on.lock().map(|g| *g).unwrap_or(false) {
            let _ = sysproxy::disable();
            *proxy_on.lock().unwrap() = false;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(core_status_raw(&state))
}

// ---- subscription -------------------------------------------------------

#[tauri::command]
async fn subscribe(
    state: State<'_, AppState>,
    url: String,
    headers: Option<HashMap<String, String>>,
    save_name: Option<String>,
) -> Result<subscribe::SubscribeOutcome, String> {
    let headers = headers.unwrap_or_default();
    let ctl = state.ctl.clone();
    let client = ctl.client().clone();
    let (body, content_type, userinfo) = fetch_subscribe(&client, &url, &headers).await?;
    let text = String::from_utf8_lossy(&body).into_owned();

    let parsed = tauri::async_runtime::spawn_blocking(move || ctl.parse(&text))
        .await
        .map_err(|e| format!("parse task failed: {e}"))??;

    if save_name.is_some() && parsed.nodes.is_empty() {
        let detail = parsed
            .errors
            .first()
            .map(|e| e.reason.as_str())
            .unwrap_or("未知原因");
        return Err(format!(
            "未能从该订阅解析出节点（{} 错误），未创建订阅：{detail}",
            parsed.errors.len()
        ));
    }

    let kept = filter_nodes(&parsed.nodes, state.settings().filter_ipv6);
    if kept.is_empty() && !parsed.nodes.is_empty() && save_name.is_some() {
        return Err("「过滤 IPv6 节点」已开启：该订阅没有可用节点，未创建订阅".into());
    }

    let group_id = if let Some(name) = save_name {
        let db = state.db.clone();
        let (nodes, userinfo_json) = {
            let nodes: Vec<NewNode> = kept
                .iter()
                .map(|n| NewNode {
                    id: n.id.clone(),
                    r#type: n.r#type.clone(),
                    remark: n.remark.clone(),
                    out: n.out.to_string(),
                })
                .collect();
            let ui = userinfo
                .as_ref()
                .map(|u| serde_json::to_string(u).unwrap_or_default());
            (nodes, ui)
        };
        // split UA out of the header map for the dedicated column
        let mut extras = headers.clone();
        let ua = extras.remove("User-Agent").filter(|v| !v.trim().is_empty());
        let extras_json = if extras.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&extras).unwrap_or_default())
        };
        let url_for_db = url.clone();
        let ua2 = ua.clone();
        let extras_json2 = extras_json.clone();
        let userinfo_json2 = userinfo_json.clone();
        Some(
            tauri::async_runtime::spawn_blocking(move || -> Result<i64, String> {
                let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
                let gid = db
                    .create_group(&name, Some(&url_for_db))
                    .map_err(|e| e.to_string())?;
                db.upsert_nodes(gid, &nodes).map_err(|e| e.to_string())?;
                db.update_group_submeta(
                    gid,
                    Some(&url_for_db),
                    ua2.as_deref(),
                    extras_json2.as_deref(),
                    userinfo_json2.as_deref(),
                )
                .map_err(|e| e.to_string())?;
                Ok(gid)
            })
            .await
            .map_err(|e| e.to_string())??,
        )
    } else {
        None
    };

    Ok(SubscribeOutcome {
        url,
        content_type,
        userinfo,
        group_id,
        parsed: ImportResult { nodes: kept, errors: parsed.errors },
    })
}

// ---- tray click handling -------------------------------------------------

/// How close two left clicks on the tray icon must be to count as a double
/// click (Windows double-click time).
const TRAY_DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// AppIndicator/StatusNotifier on Linux has no native double-click event, so
/// clicks are timed here. `gen` invalidates deferred single-click actions
/// when a second click turns the gesture into a double click.
#[derive(Default)]
struct TrayClicks {
    last: Mutex<Option<Instant>>,
    gen: AtomicU64,
}

fn tray_show(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn tray_is_visible(app: &tauri::AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// Double click: open the window when hidden, hide it back to the tray when
/// visible.
fn tray_toggle(app: &tauri::AppHandle) {
    if tray_is_visible(app) {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.hide();
        }
    } else {
        tray_show(app);
    }
}

/// One "primary activation" of the tray icon (a left button click on
/// Windows/macOS, an SNI `activate` signal on Linux). A single activation
/// shows & focuses the window; a second activation within
/// [`TRAY_DOUBLE_CLICK`] toggles it (hide when visible, show when hidden).
fn on_tray_primary_activation(app: &tauri::AppHandle, clicks: &Arc<TrayClicks>) {
    let now = Instant::now();
    let (is_double, gen) = {
        let mut last = clicks.last.lock().unwrap();
        let is_double = last.is_some_and(|t| now.duration_since(t) <= TRAY_DOUBLE_CLICK);
        *last = Some(now);
        let gen = clicks.gen.fetch_add(1, Ordering::SeqCst) + 1;
        (is_double, gen)
    };
    if is_double {
        // Double activation: open or hide the window.
        tray_toggle(app);
    } else if tray_is_visible(app) {
        // Already shown: a plain activation just focuses.
        tray_show(app);
    } else {
        // Hidden: defer the show past the double-activation window; a second
        // activation supersedes it via `gen`.
        let clicks = clicks.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(TRAY_DOUBLE_CLICK + Duration::from_millis(30));
            if clicks.gen.load(Ordering::SeqCst) == gen {
                tray_show(&app);
            }
        });
    }
}

// ---- app bootstrap ------------------------------------------------------

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle();
            let state = AppState::open(handle)?;
            let app_state = state.clone();
            app.manage(state);
            // subscription auto-refresh scheduler (runs while the app lives)
            tauri::async_runtime::spawn(auto_update_loop(app_state.clone()));

            // Apply persisted desktop-integration settings at launch
            // (best effort: failures are logged, not fatal).
            if app_state.settings().auto_start {
                if let Err(e) = autostart::enable() {
                    eprintln!("autostart: {e}");
                }
            }

            let tray_icon = app
                .default_window_icon()
                .cloned()
                .unwrap_or_else(|| tauri::include_image!("icons/icon.png"));
            let toggle = MenuItem::with_id(handle, "toggle", "显示/隐藏主界面", true, None::<&str>)?;
            let quit = MenuItem::with_id(handle, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(handle, &[&toggle, &quit])?;
            let _tray = TrayIconBuilder::new()
                .icon(tray_icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "toggle" => tray_toggle(app),
                    "quit" => {
                        let state = app.state::<AppState>();
                        state.quitting.store(true, Ordering::SeqCst);
                        shutdown_all(&state);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event({
                    let clicks = Arc::new(TrayClicks::default());
                    move |tray, event| {
                        if let tauri::tray::TrayIconEvent::Click {
                            button: tauri::tray::MouseButton::Left,
                            button_state: tauri::tray::MouseButtonState::Up,
                            ..
                        } = event
                        {
                            on_tray_primary_activation(tray.app_handle(), &clicks);
                        }
                    }
                })
                .build(app)?;
            // tray-icon emits no icon-click events on Linux; wire the
            // appindicator `activate` signal so tray clicks work there too.
            #[cfg(target_os = "linux")]
            tray_linux::connect(handle, &_tray);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if !state.quitting.load(Ordering::SeqCst) && state.settings().close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            core_version,
            parse_text,
            subscribe,
            subscription_edit,
            subscription_refresh,
            groups_list,
            nodes_list,
            import_to_group,
            create_group,
            create_strategy_group,
            rename_group,
            delete_group,
            delete_node,
            route_profiles_list,
            route_profile_create,
            route_profile_update,
            route_profile_delete,
            route_profile_set_active,
            copy_nodes,
            node_encode,
            node_qr,
            settings_get,
            settings_set,
            set_node_current,
            measure_node,
            measure_batch,
            core_status,
            core_start,
            core_stop,
            proxy_set,
            log_tail,
            latency_list,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            let state = app_handle.state::<AppState>();
            shutdown_all(&state);
        }
    });
}
