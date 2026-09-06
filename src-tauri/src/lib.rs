//! Orchestrator layer: owns the core child process, storage and platform
//! capabilities. UI-facing commands live here (see ../src/api.ts for the
//! mirrored TS contracts).

mod core;
mod subscribe;

use core::CoreCtl;
use serde::Serialize;
use subscribe::{fetch_subscribe, SubUserInfo};
use tauri::State;

use crate::core::{CoreStatus, ImportResult};

#[derive(Serialize, Clone)]
pub struct SubscribeOutcome {
    pub url: String,
    pub content_type: Option<String>,
    pub userinfo: Option<SubUserInfo>,
    #[serde(flatten)]
    pub parsed: ImportResult,
}

#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

/// Version line of the embedded sing-box, via `nekos-core version`.
#[tauri::command]
fn core_version(ctl: State<'_, CoreCtl>) -> Result<String, String> {
    ctl.version()
}

/// Parse raw link/subscription text in the core process (single source of
/// protocol truth) and return normalized nodes + per-item errors.
#[tauri::command]
fn parse_text(text: String, ctl: State<'_, CoreCtl>) -> Result<ImportResult, String> {
    ctl.parse(&text)
}

/// Orchestrator-level core status. The JSON-RPC control channel (and with
/// it real status) is a P0 follow-up; until then this reports the manager
/// as idle.
#[tauri::command]
fn core_status() -> CoreStatus {
    CoreStatus {
        running: false,
        started_at: None,
    }
}

/// Fetch a subscription URL, then parse its body in the core process.
/// Parsing runs on a blocking thread (child-process I/O). Optional custom
/// request headers support providers that require a specific User-Agent.
#[tauri::command]
async fn subscribe(
    url: String,
    headers: Option<std::collections::HashMap<String, String>>,
    ctl: State<'_, CoreCtl>,
) -> Result<SubscribeOutcome, String> {
    let headers = headers.unwrap_or_default();
    let (body, content_type, userinfo) =
        fetch_subscribe(ctl.inner().client(), &url, &headers).await?;
    let text = String::from_utf8_lossy(&body).into_owned();
    let ctl = ctl.inner().clone();
    let parsed = tauri::async_runtime::spawn_blocking(move || ctl.parse(&text))
        .await
        .map_err(|e| format!("parse task failed: {e}"))??;
    Ok(SubscribeOutcome { url, content_type, userinfo, parsed })
}

pub fn run() {
    tauri::Builder::default()
        .manage(CoreCtl::new())
        .invoke_handler(tauri::generate_handler![
            ping,
            core_version,
            parse_text,
            core_status,
            subscribe
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
