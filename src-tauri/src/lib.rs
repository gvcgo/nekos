//! Orchestrator layer: owns the core child process, storage and platform
//! capabilities. UI-facing commands live here (see ../src/api.ts for the
//! mirrored TS contracts).

mod core;

use core::CoreCtl;
use tauri::State;

use crate::core::{CoreStatus, ImportResult};

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

pub fn run() {
    tauri::Builder::default()
        .manage(CoreCtl::new())
        .invoke_handler(tauri::generate_handler![
            ping,
            core_version,
            parse_text,
            core_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
