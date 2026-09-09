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
//!
//! Lifecycle hardening: the daemon is spawned with `--parent-pid <gui>` and
//! exits by itself if it gets reparented (the GUI died without running
//! shutdown), and the app reaps any orphaned `nekos-core serve` processes at
//! startup — an orphan would otherwise keep the inbound port and the
//! system-proxy route, blocking a fresh GUI from starting or switching nodes.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
#[cfg(test)]
use std::sync::LazyLock;
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
        let mut cmd = Command::new(bin);
        cmd.arg("serve")
            .arg("--rpc")
            .arg("127.0.0.1:0")
            .arg("--token")
            .arg(&self.token);
        // The daemon watches this pid and exits if it gets reparented (the
        // GUI died without running shutdown): a crashed GUI must not leave
        // an orphaned control process holding the inbound port.
        #[cfg(unix)]
        cmd.arg("--parent-pid").arg(std::process::id().to_string());
        let mut child = cmd
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

/// Kill `nekos-core serve` control processes that outlived their GUI
/// (startup cleanup, Linux). A crashed GUI leaves its daemon — and the
/// sing-box instance inside it, which still owns the inbound port — running;
/// a fresh GUI would otherwise fail to start or switch nodes. This is
/// belt-and-braces next to the daemon's own parent-watch: it also clears
/// daemons started by older core builds that predate the watch flag.
/// No-op elsewhere.
pub fn reap_orphan_daemons() {
    #[cfg(target_os = "linux")]
    {
        use std::fs;

        fn matches_core_serve(pid: i64) -> bool {
            let Ok(raw) = fs::read(format!("/proc/{pid}/cmdline")) else {
                return false;
            };
            let mut is_core = false;
            let mut has_serve = false;
            for arg in raw.split(|&b| b == 0) {
                if arg.is_empty() {
                    continue;
                }
                if arg == b"serve" {
                    has_serve = true;
                }
                if arg.ends_with(b"nekos-core") {
                    is_core = true;
                }
            }
            is_core && has_serve
        }

        let mut pids: Vec<i32> = Vec::new();
        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let Ok(pid) = entry.file_name().to_string_lossy().parse::<i64>() else {
                    continue;
                };
                if pid == std::process::id() as i64 {
                    continue;
                }
                if matches_core_serve(pid) {
                    pids.push(pid as i32);
                }
            }
        }
        if pids.is_empty() {
            return;
        }
        for pid in &pids {
            unsafe {
                libc::kill(*pid, libc::SIGTERM);
            }
        }
        // Give them a moment to shut down, then force-kill survivors.
        std::thread::sleep(Duration::from_millis(300));
        for pid in pids {
            unsafe {
                if libc::kill(pid, 0) == 0 {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }
}

/// Serializes tests that spawn or kill real `nekos-core serve` processes so
/// the orphan reaper never kills another test's live daemon (cargo runs the
/// #[test]s of a binary in parallel threads).
#[cfg(test)]
pub(crate) static DAEMON_TEST_LOCK: LazyLock<parking_lot::Mutex<()>> =
    LazyLock::new(|| parking_lot::Mutex::new(()));

#[cfg(test)]
pub(crate) fn daemon_test_lock<'a>() -> parking_lot::MutexGuard<'a, ()> {
    DAEMON_TEST_LOCK.lock()
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

    /// The startup reaper must kill orphaned `nekos-core serve` daemons
    /// (left by a crashed GUI) and leave unrelated processes alone.
    #[cfg(target_os = "linux")]
    #[test]
    fn orphan_reap_only_kills_core_daemons() {
        let _g = crate::runtime::daemon_test_lock();
        use std::path::PathBuf;
        let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../core/bin/nekos-core");
        assert!(
            bin.is_file(),
            "core binary missing — run ./build.sh core first"
        );
        let dir = std::env::temp_dir().join(format!("nekos-reap-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let daemon_pid = dir.join("daemon.pid");
        let sleeper_pid = dir.join("sleeper.pid");

        // Orphan a real daemon: the wrapper shell exits right after
        // backgrounding it, so the daemon is reparented (parent gone).
        let script = format!(
            "'{}' serve --rpc 127.0.0.1:0 --token reap-test >/dev/null 2>&1 & echo $! > '{}'",
            bin.display(),
            daemon_pid.display()
        );
        assert!(std::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .status()
            .unwrap()
            .success());
        // An unrelated orphaned process that must survive the reaper.
        let script = format!(
            "sleep 30 >/dev/null 2>&1 & echo $! > '{}'",
            sleeper_pid.display()
        );
        assert!(std::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .status()
            .unwrap()
            .success());

        let read_pid = |p: &std::path::Path| -> i32 {
            std::fs::read_to_string(p)
                .unwrap_or_else(|_| panic!("pid file missing: {}", p.display()))
                .trim()
                .parse()
                .expect("pid number")
        };
        let alive = |pid: i32| std::path::Path::new(&format!("/proc/{pid}")).exists();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if daemon_pid.exists()
                && sleeper_pid.exists()
                && alive(read_pid(&daemon_pid))
                && alive(read_pid(&sleeper_pid))
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "test processes did not come up"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let dpid = read_pid(&daemon_pid);
        let spid = read_pid(&sleeper_pid);

        super::reap_orphan_daemons();

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while alive(dpid) {
            assert!(
                std::time::Instant::now() < deadline,
                "orphaned daemon survived reaper"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(alive(spid), "reaper killed an unrelated process");
        let _ = std::process::Command::new("kill")
            .arg("-9")
            .arg(spid.to_string())
            .status();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
