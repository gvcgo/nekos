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
    fn build_session(
        &self,
        group_id: i64,
        rule_assets: Option<(String, String)>,
    ) -> Result<(String, String), String> {
        let settings = self.settings();
        let db = self.db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let nodes = db.list_nodes(group_id).map_err(|e| format!("db: {e}"))?;
        if nodes.is_empty() {
            return Err("当前分组没有节点".into());
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
fn is_ipv6_server(out: &serde_json::Value) -> bool {
    out.get("server")
        .and_then(|s| s.as_str())
        .and_then(|h| h.parse::<std::net::IpAddr>().ok())
        .map(|ip| ip.is_ipv6())
        .unwrap_or(false)
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

/// Measure latency through one node (https URL probe, like the CLI test).
/// The result is persisted so it survives page switches and restarts.
#[tauri::command]
async fn measure_node(
    state: State<'_, AppState>,
    group_id: i64,
    node_id: String,
) -> Result<MeasureView, String> {
    let db = state.db.clone();
    let ctl = state.ctl.clone();
    let node_id2 = node_id.clone();
    let session = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
        let node = db
            .node(group_id, &node_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "节点不存在".to_string())?;
        let out: serde_json::Value =
            serde_json::from_str(&node.out).map_err(|e| format!("node json: {e}"))?;
        Ok(serde_json::json!({
            "mode": "global",
            "entries": [{ "id": node.id, "out": out }],
            "selected": node.id,
        })
        .to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let view = match ctl.core_test(&session, "https://www.google.com/generate_204", 8.0) {
            Ok(ms) => MeasureView {
                delay_ms: Some(ms),
                error: None,
            },
            Err(msg) => MeasureView {
                delay_ms: None,
                error: Some(msg),
            },
        };
        // persist so results survive redraws / restarts
        if let Ok(db) = db.lock() {
            let _ = db.upsert_latency(
                group_id,
                &node_id2,
                view.delay_ms,
                view.error.as_deref(),
                &unix_now_secs(),
            );
        }
        view
    })
    .await
    .map_err(|e| e.to_string())
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

/// Re-fetch a subscription group using its saved URL + headers and replace
/// the group's nodes.
#[tauri::command]
async fn subscription_refresh(
    state: State<'_, AppState>,
    group_id: i64,
) -> Result<subscribe::SubscribeOutcome, String> {
    let db = state.db.clone();
    let client = state.ctl.client().clone();
    let ctl = state.ctl.clone();

    let (url, headers) = tauri::async_runtime::spawn_blocking(move || -> Result<(String, HashMap<String, String>), String> {
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
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

    let kept = filter_nodes(&parsed.nodes, state.settings().filter_ipv6);
    if kept.is_empty() && !parsed.nodes.is_empty() {
        return Err("「过滤 IPv6 节点」已开启：该订阅更新后没有可用节点，现有节点未改动".into());
    }

    let db = state.db.clone();
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
        let db = db.lock().map_err(|_| "db lock poisoned".to_string())?;
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

fn unix_now_secs() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
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
    let (session_json, selected) = state.build_session(group_id, rule_assets)?;

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
        .setup(|app| {
            let handle = app.handle();
            let state = AppState::open(handle)?;
            app.manage(state);

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
            settings_get,
            settings_set,
            set_node_current,
            measure_node,
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
