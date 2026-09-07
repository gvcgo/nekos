//! Core control-plane client. Protocol parsing, sing-box config assembly
//! and the instance lifecycle live in the Go core (single source of
//! truth); this module lazily spawns the long-lived `nekos-core serve`
//! control process (architecture.md §6.1) and talks JSON-RPC 2.0 to it
//! over 127.0.0.1 with a per-run random bearer token. Starting, stopping
//! and switching the instance are core.start/core.stop/core.status calls;
//! node switching while running is the core's in-process rebuild path.
//! The `version` banner stays a one-shot CLI call — stateless and cheap.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::runtime::{Daemon, LogEntry, RpcConn};

/// Per-run bearer token, hex-encoded random bytes.
fn new_token() -> String {
    let mut buf = [0u8; 16];
    if getrandom::getrandom(&mut buf).is_err() {
        // Last-resort fallback (should not happen on supported platforms):
        // derive something unpredictable-ish from time + pid.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        buf.copy_from_slice(&nanos.to_le_bytes());
        buf[8..].copy_from_slice(&std::process::id().to_le_bytes());
    }
    let mut s = String::with_capacity(64);
    for b in buf {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[derive(Clone)]
pub struct CoreCtl {
    bin: PathBuf,
    /// Async HTTP client for subscription fetches (20s, no_proxy).
    client: reqwest::Client,
    /// Blocking JSON-RPC transport to the control process. Sync call sites
    /// (spawn_blocking workers, background refresh) use this directly.
    rpc: reqwest::blocking::Client,
    daemon: Arc<Mutex<Daemon>>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct ParseError {
    pub line: i64,
    pub snippet: String,
    pub reason: String,
}

#[derive(Deserialize, Serialize, Clone)]
struct NodeRaw {
    id: String,
    remark: String,
    out: serde_json::Value,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct NodeMeta {
    pub id: String,
    pub remark: String,
    pub r#type: String,
    /// Full sing-box outbound options JSON (persisted verbatim).
    pub out: serde_json::Value,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct UrlTestRow {
    pub id: String,
    #[serde(default)]
    pub delay_ms: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Snapshot of one tracked batch probe (core.url_test_progress).
#[derive(Deserialize, Clone, Default)]
pub struct UrlTestProgress {
    #[serde(default)]
    pub found: bool,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub results: Vec<UrlTestRow>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct ImportResult {
    pub nodes: Vec<NodeMeta>,
    pub errors: Vec<ParseError>,
}

impl ImportResult {
    fn from_raw(raw: serde_json::Value) -> Self {
        let nodes = raw
            .get("nodes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|n| serde_json::from_value::<NodeRaw>(n.clone()).ok())
                    .map(|n| NodeMeta {
                        r#type: n
                            .out
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("?")
                            .into(),
                        id: n.id,
                        remark: n.remark,
                        out: n.out,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let errors = raw
            .get("errors")
            .and_then(|v| serde_json::from_value::<Vec<ParseError>>(v.clone()).ok())
            .unwrap_or_default();
        ImportResult { nodes, errors }
    }
}

/// Instance status mirrored from core.start/core.stop/core.status results.
#[derive(Deserialize)]
struct CoreStatus {
    running: bool,
    #[serde(default)]
    started_at: Option<String>,
}

impl CoreCtl {
    pub fn new() -> Self {
        CoreCtl {
            bin: Self::find_binary(),
            client: reqwest::Client::builder()
                .user_agent("nekos/0.1")
                // Prefer direct connections: env proxies (dev shells often
                // export http_proxy) are unreliable for subscription hosts
                // and subscriptions are generally directly reachable.
                .no_proxy()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .expect("http client build"),
            // JSON-RPC runs over loopback with the bearer token; proxying
            // must never apply. reqwest's blocking client defaults to a
            // 30s total request timeout — far too short for a batch url
            // test of a large group (probes run inside the daemon) — so
            // set a generous one explicitly.
            rpc: reqwest::blocking::Client::builder()
                .user_agent("nekos/0.1")
                .no_proxy()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .expect("rpc client build"),
            daemon: Arc::new(Mutex::new(Daemon::new(new_token()))),
        }
    }

    fn spawn_hint(&self, e: std::io::Error) -> String {
        format!(
            "cannot run core `{}` ({e}); build it with: ./build.sh (or: cd core && go build -tags with_utls,with_grpc -o bin/nekos-core ./cmd/nekos-core)",
            self.bin.display()
        )
    }

    /// Shared async HTTP client used for subscription fetching.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    fn find_binary() -> PathBuf {
        if let Ok(path) = std::env::var("NEKOS_CORE") {
            if !path.is_empty() {
                return PathBuf::from(path);
            }
        }
        // Dev layout: repo/core/bin/nekos-core (built by `go build -o`).
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dev = manifest.join("../core/bin/nekos-core");
        if dev.is_file() {
            return dev;
        }
        // Fall back to PATH so a system-installed core also works.
        PathBuf::from("nekos-core")
    }

    /// `nekos-core version` banner (one-shot CLI; does not need the daemon).
    pub fn version(&self) -> Result<String, String> {
        let out = Command::new(&self.bin)
            .arg("version")
            .output()
            .map_err(|e| self.spawn_hint(e))?;
        if !out.status.success() {
            return Err(format!(
                "core version failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Ensure the control process is up and return its RPC endpoint.
    fn conn(&self) -> Result<RpcConn, String> {
        let mut daemon = self.daemon.lock();
        daemon.conn(&self.bin)
    }

    /// One JSON-RPC call. The daemon is spawned lazily on first use and
    /// reused afterwards; the spawn holds the daemon lock only briefly
    /// (never during the network call).
    ///
    /// A connection-level failure that happens immediately (refused/reset:
    /// the daemon died between the reap check and the send) drops the stale
    /// endpoint, respawns the control process and retries once. A failure
    /// after the call had time to reach the daemon (a long probe, or a
    /// server-side stall) leaves the healthy daemon alone and surfaces the
    /// error — respawning there would kill a busy control process for
    /// nothing.
    fn rpc(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        for attempt in 0..2 {
            let conn = self.conn()?;
            let started = std::time::Instant::now();
            match rpc_call(&self.rpc, &conn, method, params.clone()) {
                Ok(v) => return Ok(v),
                Err(e)
                    if e.starts_with("core rpc send error")
                        && attempt == 0
                        && started.elapsed() < std::time::Duration::from_secs(5) =>
                {
                    let mut daemon = self.daemon.lock();
                    daemon.invalidate();
                }
                Err(e) => return Err(e),
            }
        }
        Err("core rpc unreachable after respawn".into())
    }

    /// Store the instance state returned by a lifecycle RPC.
    fn store_status(&self, v: serde_json::Value) -> Result<(), String> {
        let st: CoreStatus =
            serde_json::from_value(v).map_err(|e| format!("core status output: {e}"))?;
        let mut daemon = self.daemon.lock();
        daemon.set_instance(st.running, st.started_at);
        Ok(())
    }

    /// Start (or, when already running, switch) the instance for a session.
    pub fn core_start(&self, session_json: &str) -> Result<(), String> {
        let sess = serde_json::from_str::<serde_json::Value>(session_json)
            .map_err(|e| format!("session json: {e}"))?;
        let res = self.rpc("core.start", serde_json::json!({ "session": sess }))?;
        self.store_status(res)
    }

    /// Stop the running instance (the control process stays up).
    pub fn core_stop(&self) -> Result<(), String> {
        let res = self.rpc("core.stop", serde_json::json!({}))?;
        self.store_status(res)
    }

    /// Mirrored instance state: (running, started_at). Local view only —
    /// refreshed by every start/stop; no daemon is spawned for this.
    pub fn mirror(&self) -> (bool, Option<String>) {
        self.daemon.lock().status()
    }

    /// Last N captured core log lines (newest last).
    pub fn tail_logs(&self, limit: usize) -> Vec<LogEntry> {
        self.daemon.lock().tail_logs(limit)
    }

    /// Kill the control process (app quit). The instance dies with it.
    pub fn shutdown(&self) {
        self.daemon.lock().shutdown();
    }

    /// `parse.text`: parse arbitrary link/subscription text.
    pub fn parse(&self, text: &str) -> Result<ImportResult, String> {
        let res = self.rpc("parse.text", serde_json::json!({ "text": text }))?;
        Ok(ImportResult::from_raw(res))
    }

    /// `core.url_test`: batch-probe every entry in one core instance
    /// (v2rayN semantics). Returns rows in entry order.
    pub fn urltest(&self, session_json: &str) -> Result<Vec<UrlTestRow>, String> {
        let params = serde_json::from_str::<serde_json::Value>(session_json)
            .map_err(|e| format!("urltest session json: {e}"))?;
        let res = self.rpc("core.url_test", params)?;
        serde_json::from_value(res).map_err(|e| format!("core url_test output: {e}"))
    }

    /// `core.url_test_progress`: poll one tracked batch run (started by
    /// passing "run_id" in the url_test session). Returns the rows that
    /// finished so far; done=true once the run completed (the run is then
    /// consumed by the server). found=false when the server does not know
    /// the run yet (call again shortly).
    pub fn url_test_progress(&self, run_id: &str) -> Result<UrlTestProgress, String> {
        let res = self.rpc(
            "core.url_test_progress",
            serde_json::json!({ "run_id": run_id }),
        )?;
        serde_json::from_value(res)
            .map_err(|e| format!("core url_test_progress output: {e}"))
    }

    /// `encode`: produce a share link from a node's outbound JSON + remark.
    pub fn encode(&self, remark: &str, out: &serde_json::Value) -> Result<String, String> {
        let res = self.rpc(
            "encode",
            serde_json::json!({ "remark": remark, "out": out }),
        )?;
        Ok(res.as_str().map(|s| s.to_string()).unwrap_or_default())
    }

    /// `qr`: base64 PNG QR of a node's share link.
    pub fn qr(&self, remark: &str, out: &serde_json::Value, size: u32) -> Result<String, String> {
        let res = self.rpc(
            "qr",
            serde_json::json!({ "remark": remark, "out": out, "size": size }),
        )?;
        Ok(res.as_str().map(|s| s.to_string()).unwrap_or_default())
    }
}

impl Default for CoreCtl {
    fn default() -> Self {
        Self::new()
    }
}

/// POST one JSON-RPC 2.0 request and return the result payload.
/// HTTP status and rpc error objects are folded into Err messages.
fn rpc_call(
    client: &reqwest::blocking::Client,
    conn: &RpcConn,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let resp = client
        .post(format!("{}/rpc", conn.url))
        .bearer_auth(&conn.token)
        .json(&body)
        .send()
        .map_err(|e| format!("core rpc send error for {method}: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| format!("core rpc {method}: read response: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "core rpc {method} failed: HTTP {status} {}",
            text.trim()
        ));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("core rpc {method}: malformed response {text:?}: {e}"))?;
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(format!("core rpc {method}: {msg}"));
    }
    v.get("result")
        .cloned()
        .ok_or_else(|| format!("core rpc {method}: response without result: {text:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// Full lifecycle over JSON-RPC against the real `serve` control
    /// process (no network needed: outbound init is lazy). Requires
    /// `core/bin/nekos-core` (built with ./build.sh core).
    #[test]
    fn rpc_lifecycle_via_real_daemon() {
        let ctl = CoreCtl::new();
        // First call spawns the daemon lazily and parses over RPC.
        let parsed = ctl
            .parse("anytls://e0c664d9-415f-30b5-aeaf-547be3870274@ew.ali66mysql.com:26019/?sni=www.apple.com&insecure=1#integration-test")
            .expect("parse");
        assert_eq!(parsed.nodes.len(), 1);
        let n = &parsed.nodes[0];
        let session = serde_json::json!({
            "mode": "global",
            "inbound": { "listen": "127.0.0.1", "port": free_port(), "type": "mixed" },
            "entries": [{ "id": n.id, "out": n.out }],
            "selected": n.id,
        })
        .to_string();

        assert!(!ctl.mirror().0);
        ctl.core_start(&session).expect("core start");
        let (running, started_at) = ctl.mirror();
        assert!(running);
        assert!(started_at.is_some());

        // Switching node while running = core.start again (in-process
        // rebuild, not a process restart).
        ctl.core_start(&session).expect("core start (rebuild)");
        assert!(ctl.mirror().0);

        ctl.core_stop().expect("core stop");
        assert!(!ctl.mirror().0);
        // stop is idempotent over RPC
        ctl.core_stop().expect("core stop (idempotent)");

        // Daemon is still up after the instance stopped: encode over RPC
        // proves the control process outlives the instance.
        let link = ctl.encode(&n.remark, &n.out).expect("encode");
        assert!(link.starts_with("anytls://"), "link: {link}");
        ctl.shutdown();
    }

    /// The daemon must reject requests without the right bearer token.
    #[test]
    fn rpc_auth_enforced() {
        let ctl = CoreCtl::new();
        let conn = ctl.conn().expect("daemon conn");
        let wrong = RpcConn {
            url: conn.url.clone(),
            token: "wrong-token".into(),
        };
        let err = rpc_call(&ctl.rpc, &wrong, "core.status", serde_json::json!({}))
            .expect_err("must fail");
        assert!(err.contains("401"), "err: {err}");
        // correct token works
        rpc_call(&ctl.rpc, &conn, "core.status", serde_json::json!({})).expect("status");
        ctl.shutdown();
    }
}
