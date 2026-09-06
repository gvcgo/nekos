//! Orchestrator layer: SQLite storage, core child supervision, system
//! proxy, tray and all UI-facing commands (see ../src/api.ts for the
//! mirrored TS contracts).

mod core;
mod db;
mod runtime;
mod subscribe;
mod sysproxy;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use db::{Db, NewNode, Settings};
use runtime::RuntimeState;
use serde::Serialize;
use subscribe::{fetch_subscribe, SubscribeOutcome};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, State};

use crate::core::{CoreCtl, ImportResult};

// ---- shared state -------------------------------------------------------

#[derive(Clone)]
pub struct AppState {
    db: Arc<Mutex<Db>>,
    ctl: CoreCtl,
    runtime: Arc<Mutex<RuntimeState>>,
    proxy_on: Arc<Mutex<bool>>,
    quitting: Arc<AtomicBool>,
    updating: Arc<Mutex<std::collections::HashSet<i64>>>,
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
            runtime: Arc::new(Mutex::new(RuntimeState::default())),
            proxy_on: Arc::new(Mutex::new(false)),
            quitting: Arc::new(AtomicBool::new(false)),
            updating: Arc::new(Mutex::new(std::collections::HashSet::new())),
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
        let nodes = db.list_nodes(group_id).map_err(|e| format!("db: {e}"))?;
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
            .get(&group_id)
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
        let mode = match settings.mode.as_str() {
            "direct" => "direct",
            "rule" => "rule",
            _ => "global",
        };
        let mut session = serde_json::json!({
            "mode": mode,
            "inbound": { "listen": "127.0.0.1", "port": settings.port, "type": "mixed" },
            "entries": entries,
            "selected": selected,
            "log_level": settings.log_level,
        });
        if mode == "rule" {
            let (ip, site) = rule_assets
                .ok_or_else(|| "规则模式需要 CN 规则集，请重试以触发下载".to_string())?;
            session["rule_assets"] = serde_json::json!({
                "geoip_cn": ip,
                "geosite_cn": site,
            });
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
    if let Ok(mut rt) = state.runtime.lock() {
        let _ = rt.stop();
    }
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
fn parse_text(text: String, ctl: State<'_, CoreCtl>) -> Result<ImportResult, String> {
    ctl.parse(&text)
}

#[tauri::command]
async fn groups_list(state: State<'_, AppState>) -> Result<Vec<db::Group>, String> {
    with_db(&state, |db| db.list_groups().map_err(|e| e.to_string()))
}

#[tauri::command]
async fn nodes_list(state: State<'_, AppState>, group_id: i64) -> Result<Vec<db::Node>, String> {
    with_db(&state, |db| {
        db.list_nodes(group_id).map_err(|e| e.to_string())
    })
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
}

#[tauri::command]
async fn settings_set(
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> Result<Settings, String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
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
        db.save_settings(&s).map_err(|e| e.to_string())?;
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Last captured core log lines (newest last).
#[tauri::command]
fn log_tail(state: State<'_, AppState>, limit: Option<usize>) -> Vec<runtime::LogEntry> {
    state
        .runtime
        .lock()
        .map(|rt| rt.tail_logs(limit.unwrap_or(300)))
        .unwrap_or_default()
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
        if db
            .node(group_id, &node_id)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Err("节点不存在".to_string());
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

/// Build the urltest session for a node subset of a group and probe it in
/// one sing-box instance (v2rayN semantics: concurrent, HTTP 204).
fn run_urltest(
    db: &Mutex<Db>,
    ctl: &CoreCtl,
    group_id: i64,
    ids: Option<&[String]>,
    filter_v6: bool,
) -> Result<Vec<core::UrlTestRow>, String> {
    let pairs = {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let nodes = db.list_nodes(group_id).map_err(|e| format!("db: {e}"))?;
        let selected: Vec<(String, serde_json::Value)> = nodes
            .iter()
            .filter(|n| ids.map(|ids| ids.contains(&n.id)).unwrap_or(true))
            .filter_map(|n| {
                let out: serde_json::Value = serde_json::from_str(&n.out).ok()?;
                if filter_v6 && is_ipv6_server(&out) {
                    return None;
                }
                Some((n.id.clone(), out))
            })
            .collect();
        if selected.is_empty() {
            return Err("没有可测速的节点".into());
        }
        selected
    };
    let entries: Vec<serde_json::Value> = pairs
        .into_iter()
        .map(|(id, out)| serde_json::json!({ "id": id, "out": out }))
        .collect();
    let session = serde_json::json!({ "entries": entries, "timeout_s": 5 }).to_string();
    ctl.urltest(&session)
}

fn persist_rows(db: &Mutex<Db>, group_id: i64, rows: &[core::UrlTestRow]) {
    if let Ok(db) = db.lock() {
        for r in rows {
            let _ = db.upsert_latency(
                group_id,
                &r.id,
                r.delay_ms,
                r.error.as_deref(),
                &unix_now_secs(),
            );
        }
    }
}

/// Measure one node (single-entry batch, same engine as batch testing).
#[tauri::command]
async fn measure_node(
    state: State<'_, AppState>,
    group_id: i64,
    node_id: String,
) -> Result<MeasureView, String> {
    let db = state.db.clone();
    let ctl = state.ctl.clone();
    let ids = vec![node_id.clone()];
    let rows = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<core::UrlTestRow>, String> {
        let rows = run_urltest(&db, &ctl, group_id, Some(&ids), false)?;
        persist_rows(&db, group_id, &rows);
        Ok(rows)
    })
    .await
    .map_err(|e| e.to_string())??;
    let row = rows.into_iter().next().unwrap_or(core::UrlTestRow {
        id: node_id,
        delay_ms: None,
        error: Some("没有返回结果".into()),
    });
    Ok(MeasureView { delay_ms: row.delay_ms, error: row.error })
}

/// Measure every node of a group in one instance, persisted per node.
#[tauri::command]
async fn measure_batch(
    state: State<'_, AppState>,
    group_id: i64,
) -> Result<Vec<BatchRow>, String> {
    let db = state.db.clone();
    let ctl = state.ctl.clone();
    let filter_v6 = state.settings().filter_ipv6;
    let rows = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<core::UrlTestRow>, String> {
        let rows = run_urltest(&db, &ctl, group_id, None, filter_v6)?;
        persist_rows(&db, group_id, &rows);
        Ok(rows)
    })
    .await
    .map_err(|e| e.to_string())??;
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
        let (inserted, duplicated) = db
            .copy_group_nodes(source_group_id, target_group_id, &node_ids)
            .map_err(|e| e.to_string())?;
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

/// Persisted latency results for a group (shown until re-tested).
#[tauri::command]
async fn latency_list(
    state: State<'_, AppState>,
    group_id: i64,
) -> Result<Vec<db::LatencyRow>, String> {
    with_db(&state, |db| db.list_latency(group_id).map_err(|e| e.to_string()))
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
    let (running, started_at) = state
        .runtime
        .lock()
        .map(|rt| (rt.is_running(), rt.started_at().map(|s| s.to_string())))
        .unwrap_or((false, None));
    let proxy_enabled = state.proxy_on.lock().map(|g| *g).unwrap_or(false);
    CoreStatusView {
        running,
        started_at,
        proxy_enabled,
    }
}

#[tauri::command]
async fn core_start(state: State<'_, AppState>) -> Result<RunResult, String> {
    let settings = state.settings();
    let group_id = settings.current_group_id;
    let rule_assets = if settings.mode == "rule" {
        Some(state.ensure_rule_assets().await?)
    } else {
        None
    };
    let (session_json, selected) =
        state.build_session(group_id, rule_assets, settings.filter_ipv6)?;

    let ctl = state.ctl.clone();
    let runtime = state.runtime.clone();
    // System proxy is NOT auto-enabled on start: the user controls it
    // independently via the toolbar switch (proxy_set).
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut rt = runtime
            .lock()
            .map_err(|_| "runtime lock poisoned".to_string())?;
        rt.start(&ctl, &session_json)
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
    let running = state.runtime.lock().map(|rt| rt.is_running()).unwrap_or(false);
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
    let runtime = state.runtime.clone();
    let proxy_on = state.proxy_on.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        if let Ok(mut rt) = runtime.lock() {
            rt.stop()?;
        }
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
            tauri::async_runtime::spawn(auto_update_loop(app_state));

            let tray_icon = app
                .default_window_icon()
                .cloned()
                .unwrap_or_else(|| tauri::include_image!("icons/icon.png"));
            let show = MenuItem::with_id(handle, "show", "显示主界面", true, None::<&str>)?;
            let quit = MenuItem::with_id(handle, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(handle, &[&show, &quit])?;
            let _tray = TrayIconBuilder::new()
                .icon(tray_icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => {
                        let state = app.state::<AppState>();
                        state.quitting.store(true, Ordering::SeqCst);
                        shutdown_all(&state);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;
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
            rename_group,
            delete_group,
            delete_node,
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
