//! Long-lived core control-process supervision and the JSON-RPC transport
//! to it (architecture.md §6.1/§6.2). The orchestrator spawns
//! `nekos-core serve --rpc 127.0.0.1:0 --token <token>` lazily on the first
//! RPC need, reads the bound address from its first stdout line
//! ({"rpc": "127.0.0.1:PORT"}), and keeps the process for the app's
//! lifetime. All control — parse, start/stop/status, batch url tests,
//! encode — is JSON-RPC over loopback with a bearer token; the sing-box
//! instance lives inside that process, so switching nodes is the in-process
//! rebuild path (core.start while running → Manager.Replace), not a
//! process kill + respawn. The daemon's stderr is streamed into a log ring
//! exactly as before.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;

/// Strip ANSI CSI sequences (e.g. sing-box log colors if a build ever
/// re-enables them) so classification and display see plain text.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // consume until a final byte of a CSI sequence ("[..m")
            if chars.clone().next() == Some('[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// One captured core log line.
#[derive(Serialize, Clone)]
pub struct LogEntry {
    pub level: String, // error | warn | info | debug | other
    pub line: String,
}

const LOG_CAP: usize = 1000;

/// Classify the leading sing-box log level token ("ERROR[0000] ...").
fn level_of(line: &str) -> &'static str {
    let token = line.trim_start();
    let token = token.split(['[', ' ']).next().unwrap_or("");
    match token {
        "ERROR" | "FATAL" | "PANIC" => "error",
        "WARN" => "warn",
        "INFO" => "info",
        "DEBUG" => "debug",
        _ => "other",
    }
}

/// A ready-to-use RPC endpoint (base URL + bearer token).
#[derive(Clone)]
pub struct RpcConn {
    pub url: String,
    pub token: String,
}

/// The spawned `serve` process plus the instance state mirrored from RPC
/// results (core.status is authoritative; the mirror is refreshed on every
/// start/stop and on status reads).
pub struct Daemon {
    child: Option<Child>,
    /// Base URL like "http://127.0.0.1:41234"; Some once the daemon is up.
    url: Option<String>,
    token: String,
    running: bool,
    started_at: Option<String>,
    logs: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl Daemon {
    pub fn new(token: String) -> Self {
        Daemon {
            child: None,
            url: None,
            token,
            running: false,
            started_at: None,
            logs: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Reap the child if it has exited and reset the connection state so
    /// the next RPC call respawns it. Cheap enough for status reads.
    fn reap_if_exited(&mut self) {
        if let Some(child) = &mut self.child {
            if matches!(child.try_wait(), Ok(Some(_))) {
                self.child = None;
                self.url = None;
                self.running = false;
                self.started_at = None;
            }
        }
    }

    /// Spawn `nekos-core serve` unless one is already up, then return the
    /// RPC connection. Fails fast when the binary is missing or the daemon
    /// does not announce its listener within the deadline (stderr is
    /// surfaced). The caller holds the Daemon mutex across this call (it
    /// happens at most once per daemon lifetime).
    pub fn conn(&mut self, bin: &Path) -> Result<RpcConn, String> {
        self.reap_if_exited();
        if let Some(url) = self.url.clone() {
            return Ok(RpcConn {
                url,
                token: self.token.clone(),
            });
        }
        let mut child = Command::new(bin)
            .arg("serve")
            .arg("--rpc")
            .arg("127.0.0.1:0")
            .arg("--token")
            .arg(&self.token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!(
                    "cannot run core `{}` ({e}); build it with: ./build.sh (or: cd core && go build -tags with_utls,with_grpc -o bin/nekos-core ./cmd/nekos-core)",
                    bin.display()
                )
            })?;
        let stdout = child.stdout.take().expect("stdout piped");
        let ready = match read_ready(stdout, Duration::from_secs(15)) {
            Ok(line) => line,
            Err(detail) => {
                let _ = child.kill();
                let _ = child.wait();
                let stderr = capture_stderr(&mut child);
                return Err(format!("core serve failed: {detail} ({})", stderr.trim()));
            }
        };
        let addr = serde_json::from_str::<serde_json::Value>(&ready)
            .ok()
            .and_then(|v| v.get("rpc")?.as_str().map(|s| s.to_string()));
        let Some(addr) = addr else {
            // Malformed announcement: the daemon must be down, not merely
            // confused, before its stderr can be drained.
            let _ = child.kill();
            let _ = child.wait();
            let stderr = capture_stderr(&mut child);
            return Err(format!(
                "core serve: unexpected ready line {ready:?} ({})",
                stderr.trim()
            ));
        };
        let url = format!("http://{addr}");

        if let Some(err_pipe) = child.stderr.take() {
            let logs = self.logs.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(err_pipe);
                for line in reader.lines().map_while(Result::ok) {
                    let line = strip_ansi(&line);
                    if line.trim().is_empty() {
                        continue;
                    }
                    let level = level_of(&line).to_string();
                    let mut logs = logs.lock();
                    if logs.len() >= LOG_CAP {
                        logs.pop_front();
                    }
                    logs.push_back(LogEntry { level, line });
                }
            });
        }
        self.child = Some(child);
        self.url = Some(url.clone());
        self.running = false;
        self.started_at = None;
        Ok(RpcConn {
            url,
            token: self.token.clone(),
        })
    }

    /// Mirrored instance state: (running, started_at).
    pub fn instance(&self) -> (bool, Option<String>) {
        (self.running, self.started_at.clone())
    }

    /// Drop a stale/dead control process: kill any leftover child and
    /// clear the endpoint + instance mirror so the next call respawns the
    /// daemon. Used after a transport-level RPC failure.
    pub fn invalidate(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.url = None;
        self.running = false;
        self.started_at = None;
    }

    /// Update the mirrored instance state from a core.status/core.start
    /// result.
    pub fn set_instance(&mut self, running: bool, started_at: Option<String>) {
        self.running = running;
        self.started_at = started_at;
    }

    /// Reap an exited daemon and report the mirrored instance state.
    pub fn status(&mut self) -> (bool, Option<String>) {
        self.reap_if_exited();
        self.instance()
    }

    /// Last N captured log lines (newest last), empty when idle.
    pub fn tail_logs(&self, limit: usize) -> Vec<LogEntry> {
        let logs = self.logs.lock();
        let skip = logs.len().saturating_sub(limit.max(1));
        logs.iter().skip(skip).cloned().collect()
    }

    /// Kill the daemon process (app shutdown / quit). The instance dies
    /// with the process; no RPC stop is needed.
    pub fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.url = None;
        self.running = false;
        self.started_at = None;
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // Safety net: tests and early-return paths may drop the last
        // CoreCtl without an explicit shutdown — never leak the child.
        self.shutdown();
    }
}

/// Read the daemon's first stdout line (the {"rpc": ...} ready line).
/// Errors (EOF, deadline, read failure) are reported without touching
/// stderr — the caller kills the child first and drains stderr itself.
fn read_ready(mut stdout: ChildStdout, timeout: Duration) -> Result<String, String> {
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = [0u8; 1024];
    let mut ready = String::new();
    loop {
        if std::time::Instant::now() > deadline {
            return Err("core did not become ready within 15s".into());
        }
        match stdout.read(&mut buf) {
            Ok(0) => break, // EOF without a ready line
            Ok(n) => {
                ready.push_str(&String::from_utf8_lossy(&buf[..n]));
                if ready.contains('\n') {
                    break;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("read core stdout: {e}")),
        }
    }
    if ready.trim().is_empty() {
        return Err("core exited before becoming ready".into());
    }
    Ok(ready.trim().to_string())
}

fn capture_stderr(child: &mut Child) -> String {
    let mut out = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_classification() {
        assert_eq!(level_of("ERROR[0000] boom"), "error");
        assert_eq!(level_of("FATAL[0001] x"), "error");
        assert_eq!(level_of("WARN[0002] noisy"), "warn");
        assert_eq!(level_of("INFO[0003] started"), "info");
        assert_eq!(level_of("DEBUG[0004] trace"), "debug");
        assert_eq!(level_of("random line"), "other");
    }

    #[test]
    fn ansi_stripping() {
        assert_eq!(
            strip_ansi("\u{1b}[36mINFO\u{1b}[0m[0004] hello"),
            "INFO[0004] hello"
        );
        assert_eq!(strip_ansi("plain"), "plain");
        assert_eq!(strip_ansi("\u{1b}[1;31merror\u{1b}[0m"), "error");
    }
}
