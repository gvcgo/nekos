//! Long-running core child supervision. The orchestrator spawns
//! `nekos-core run` with a session, waits for its ready line, and kills it
//! on stop (process-level teardown is safe: listeners die with the
//! process). Switching nodes = stop + start with a new session, matching
//! the "instance rebuild" path documented in architecture.md §6.3.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

use crate::core::CoreCtl;

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

#[derive(Default)]
pub struct RuntimeState {
    child: Option<Child>,
    started_at: Option<String>,
    logs: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl RuntimeState {
    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn started_at(&self) -> Option<&str> {
        self.started_at.as_deref()
    }

    /// Last N captured log lines (newest last), empty when idle.
    pub fn tail_logs(&self, limit: usize) -> Vec<LogEntry> {
        let logs = self.logs.lock().unwrap();
        let skip = logs.len().saturating_sub(limit.max(1));
        logs.iter().skip(skip).cloned().collect()
    }

    /// Spawn `nekos-core run`, feed it the assembled session JSON, and wait
    /// for the ready line ({"running":true,...}). Fails fast when the core
    /// rejects the config (stderr is surfaced).
    pub fn start(&mut self, ctl: &CoreCtl, session_json: &str) -> Result<(), String> {
        if self.child.is_some() {
            return Err("core already running: stop first".into());
        }
        let mut child = Command::new(ctl.binary())
            .arg("run")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn core: {e}"))?;

        let mut stdin = child.stdin.take().expect("stdin piped");
        stdin
            .write_all(session_json.as_bytes())
            .map_err(|e| format!("write session: {e}"))?;
        drop(stdin); // EOF: core knows the session is complete

        let mut stdout = child.stdout.take().expect("stdout piped");
        let mut ready = String::new();
        // The first line is the status JSON once listeners are up.
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut buf = [0u8; 1024];
        loop {
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                return Err("core did not become ready within 15s".into());
            }
            match stdout.read(&mut buf) {
                Ok(0) => break, // EOF without ready line => startup failed
                Ok(n) => {
                    ready.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if ready.contains('\n') {
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = child.kill();
                    return Err(format!("read core stdout: {e}"));
                }
            }
        }
        if !ready.trim().starts_with('{') || !ready.contains("\"running\":true") {
            // Startup failure: capture stderr for the user.
            let mut stderr = String::new();
            if let Some(mut err_pipe) = child.stderr.take() {
                let _ = err_pipe.read_to_string(&mut stderr);
            }
            let _ = child.wait();
            let detail = if stderr.trim().is_empty() {
                "no detail".into()
            } else {
                stderr
            };
            return Err(format!("core start failed: {}", detail.trim()));
        }
        // Clear logs for a fresh run, then stream stderr line-by-line.
        self.logs.lock().unwrap().clear();
        if let Some(err_pipe) = child.stderr.take() {
            let logs = self.logs.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(err_pipe);
                for line in reader.lines().map_while(Result::ok) {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let level = level_of(&line);
                    let mut logs = logs.lock().unwrap();
                    if logs.len() >= LOG_CAP {
                        logs.pop_front();
                    }
                    logs.push_back(LogEntry { level: level.to_string(), line });
                }
            });
        }
        let started = ready
            .lines()
            .next()
            .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .and_then(|v| v.get("started_at")?.as_str().map(|s| s.to_string()));
        self.child = Some(child);
        self.started_at = started;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            self.started_at = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CoreCtl;

    #[test]
    fn log_level_classification() {
        assert_eq!(level_of("ERROR[0000] boom"), "error");
        assert_eq!(level_of("FATAL[0001] x"), "error");
        assert_eq!(level_of("WARN[0002] noisy"), "warn");
        assert_eq!(level_of("INFO[0003] started"), "info");
        assert_eq!(level_of("DEBUG[0004] trace"), "debug");
        assert_eq!(level_of("random line"), "other");
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// Boot the real core child with a parsed anytls node (no network
    /// needed: outbound init is lazy). Requires `core/bin/nekos-core`.
    #[test]
    fn core_child_start_stop() {
        let ctl = CoreCtl::new();
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

        let mut rt = RuntimeState::default();
        rt.start(&ctl, &session).expect("core start");
        assert!(rt.is_running());
        assert!(rt.started_at().is_some());
        rt.stop().expect("core stop");
        assert!(!rt.is_running());
        // idempotent stop
        rt.stop().unwrap();
    }
}
