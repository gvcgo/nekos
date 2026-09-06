//! Core child-process control. Protocol parsing and sing-box config
//! assembly live in the Go core (single source of truth); this module
//! finds, spawns and talks to the `nekos-core` binary. The JSON-RPC
//! channel (architecture.md §6) will replace the per-call CLI once the
//! long-running control surface lands.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct CoreCtl {
    bin: PathBuf,
    client: reqwest::Client,
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
        }
    }

    /// Shared HTTP client used for subscription fetching.
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

    fn run(&self, args: &[&str], stdin: &[u8]) -> Result<Vec<u8>, String> {
        let mut child = Command::new(&self.bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!(
                    "cannot run core `{}` ({e}); build it with: go build -o core/bin/nekos-core ./cmd/nekos-core",
                    self.bin.display()
                )
            })?;
        child
            .stdin
            .as_mut()
            .expect("stdin piped")
            .write_all(stdin)
            .map_err(|e| format!("write core stdin: {e}"))?;
        let out = child
            .wait_with_output()
            .map_err(|e| format!("wait core: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("core {} failed: {}", args.join(" "), stderr.trim()));
        }
        Ok(out.stdout)
    }

    /// `nekos-core version` output.
    pub fn version(&self) -> Result<String, String> {
        let out = self.run(&["version"], b"")?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// Path of the core executable (used to spawn the long-running child).
    pub fn binary(&self) -> &std::path::Path {
        &self.bin
    }

    /// `nekos-core test`: measure TCP/HTTP latency through the given
    /// session's selected node. Returns delay in milliseconds on success.
    pub fn core_test(
        &self,
        session_json: &str,
        target: &str,
        timeout_s: f64,
    ) -> Result<i64, String> {
        let secs = format!("{timeout_s}");
        let out = self.run(
            &["test", "-target", target, "-timeout", &secs],
            session_json.as_bytes(),
        )?;
        let value: serde_json::Value =
            serde_json::from_slice(&out).map_err(|e| format!("core test output: {e}"))?;
        value
            .get("delay_ms")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| format!("core test: no delay_ms in {value}"))
    }

    /// `nekos-core urltest`: batch-probe every entry in one instance
    /// (v2rayN semantics). Returns rows in entry order.
    pub fn urltest(&self, session_json: &str) -> Result<Vec<UrlTestRow>, String> {
        let out = self.run(&["urltest"], session_json.as_bytes())?;
        let rows: Vec<UrlTestRow> = serde_json::from_slice(&out)
            .map_err(|e| format!("core urltest output: {e}"))?;
        Ok(rows)
    }

    /// `nekos-core parse` over arbitrary link/subscription text.
    pub fn parse(&self, text: &str) -> Result<ImportResult, String> {
        let out = self.run(&["parse"], text.as_bytes())?;
        let raw: serde_json::Value =
            serde_json::from_slice(&out).map_err(|e| format!("core parse output: {e}"))?;
        Ok(ImportResult::from_raw(raw))
    }
}

impl Default for CoreCtl {
    fn default() -> Self {
        Self::new()
    }
}
