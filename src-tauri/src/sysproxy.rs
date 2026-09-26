//! System-proxy platform abstraction.
//!
//! - Linux (GNOME): `gsettings`.
//! - Windows: WinINET proxy keys under HKCU (takes effect system-wide for
//!   WinINET/WinHTTP consumers), then broadcasts the change so running apps
//!   notice immediately.
//! - macOS: `networksetup` per network service (web/secureweb/socksfirewall),
//!   with the applied setting read back (exit codes are not trustworthy —
//!   networksetup succeeds on services it ignores). A batch refused for lack
//!   of privileges is replayed in ONE administrator-prompted `osascript` run;
//!   other failures never prompt.
//! - Other platforms: not implemented.

#[cfg(target_os = "linux")]
mod linux_impl {
    fn gsettings(args: &[&str]) -> Result<(), String> {
        let out = std::process::Command::new("gsettings")
            .args(args)
            .output()
            .map_err(|_| "gsettings 不可用（当前桌面可能不是 GNOME）".to_string())?;
        if !out.status.success() {
            return Err(format!(
                "gsettings {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    }

    /// Route system HTTP/HTTPS/SOCKS traffic through 127.0.0.1:port.
    pub fn enable(port: u16) -> Result<(), String> {
        let host = "127.0.0.1";
        let port = &port.to_string();
        gsettings(&["set", "org.gnome.system.proxy", "mode", "manual"])?;
        for schema in [
            "org.gnome.system.proxy.http",
            "org.gnome.system.proxy.https",
        ] {
            gsettings(&["set", schema, "host", host])?;
            gsettings(&["set", schema, "port", port])?;
        }
        gsettings(&["set", "org.gnome.system.proxy.socks", "host", host])?;
        gsettings(&["set", "org.gnome.system.proxy.socks", "port", port])?;
        gsettings(&[
            "set",
            "org.gnome.system.proxy",
            "ignore-hosts",
            "['localhost', '127.0.0.0/8', '::1']",
        ])?;
        Ok(())
    }

    /// Restore the system proxy to 'none'.
    pub fn disable() -> Result<(), String> {
        gsettings(&["set", "org.gnome.system.proxy", "mode", "none"])
    }

    fn gsettings_get(schema: &str, key: &str) -> Option<String> {
        let out = std::process::Command::new("gsettings")
            .args(["get", schema, key])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout);
        Some(s.trim().trim_matches('\'').to_string())
    }

    /// True when the GNOME proxy is routed to 127.0.0.1:port in "manual"
    /// mode — i.e. a route this app enabled and a crashed run could not
    /// restore. Startup uses this to un-break traffic after an orphaned
    /// core (whose port this points at) has been reaped.
    pub fn leftover_at(port: u16) -> bool {
        let mode = gsettings_get("org.gnome.system.proxy", "mode").unwrap_or_default();
        if mode != "manual" {
            return false;
        }
        let host = gsettings_get("org.gnome.system.proxy.http", "host").unwrap_or_default();
        let port_s = gsettings_get("org.gnome.system.proxy.http", "port").unwrap_or_default();
        host == "127.0.0.1" && port_s == port.to_string()
    }
}

#[cfg(target_os = "windows")]
mod win_impl {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const HOST: &str = "127.0.0.1";
    /// WinINET settings live here for the current user.
    const INTERNET_SETTINGS: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

    fn write_registry(enable: bool, port: u16) -> Result<(), String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _disp) = hkcu
            .create_subkey(INTERNET_SETTINGS)
            .map_err(|e| format!("打开注册表 {INTERNET_SETTINGS}: {e}"))?;
        if enable {
            key.set_value("ProxyEnable", &1u32)
                .map_err(|e| format!("写入 ProxyEnable: {e}"))?;
            key.set_value("ProxyServer", &format!("{HOST}:{port}"))
                .map_err(|e| format!("写入 ProxyServer: {e}"))?;
            key.set_value("ProxyOverride", &"localhost;127.0.0.1;<local>")
                .map_err(|e| format!("写入 ProxyOverride: {e}"))?;
        } else {
            key.set_value("ProxyEnable", &0u32)
                .map_err(|e| format!("写入 ProxyEnable: {e}"))?;
        }
        Ok(())
    }

    /// Tell WinINET to re-read the settings so already-running processes pick
    /// the change up without a reboot. Best effort: the registry is the
    /// source of truth, a failed broadcast only delays non-WinINET readers.
    fn notify_change() {
        use windows_sys::Win32::Networking::WinInet::{
            InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
        };
        unsafe {
            let _ = InternetSetOptionW(
                std::ptr::null_mut(),
                INTERNET_OPTION_SETTINGS_CHANGED,
                std::ptr::null_mut(),
                0,
            );
            let _ = InternetSetOptionW(
                std::ptr::null_mut(),
                INTERNET_OPTION_REFRESH,
                std::ptr::null_mut(),
                0,
            );
        }
    }

    pub fn enable(port: u16) -> Result<(), String> {
        write_registry(true, port)?;
        notify_change();
        Ok(())
    }

    pub fn disable() -> Result<(), String> {
        write_registry(false, 0)?;
        notify_change();
        Ok(())
    }

    /// True when WinINET still routes to 127.0.0.1:port (a crash could not
    /// run disable). Startup restores it so traffic does not hit a dead
    /// orphaned core after reaping.
    pub fn leftover_at(port: u16) -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey(INTERNET_SETTINGS) else {
            return false;
        };
        let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
        if enabled == 0 {
            return false;
        }
        let server: String = key.get_value("ProxyServer").unwrap_or_default();
        server == format!("{HOST}:{port}")
    }
}

#[cfg(target_os = "macos")]
mod mac_impl {
    const HOST: &str = "127.0.0.1";
    const NETWORKSETUP: &str = "/usr/sbin/networksetup";

    /// True when networksetup refused *because the caller lacks the rights*
    /// to change network settings. Any other failure (unknown service,
    /// invalid parameters — e.g. the output banner mistake this module used
    /// to make) is a plain error and must never raise a password dialog.
    fn privilege_denied(stderr: &str) -> bool {
        let s = stderr.to_ascii_lowercase();
        [
            "must be running as root",
            "requires administrator",
            "not authorized",
            "authorization",
            "operation not permitted",
            "permission denied",
            "sudo",
        ]
        .iter()
        .any(|marker| s.contains(marker))
    }

    /// POSIX single-quote one word for the shell (`do shell script` runs
    /// `/bin/sh`); service names contain spaces and parentheses.
    fn sh_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', r"'\''"))
    }

    /// Run one networksetup argv as the current user.
    fn run_plain(args: &[String]) -> Result<(), String> {
        let out = std::process::Command::new(NETWORKSETUP)
            .args(args)
            .output()
            .map_err(|e| format!("运行 networksetup: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        Err(if err.is_empty() {
            format!("networksetup {} 退出码 {}", args.join(" "), out.status)
        } else {
            err.to_string()
        })
    }

    /// Run a whole mutation batch, escalating to ONE administrator-prompted
    /// run when — and only when — the plain run was refused for lack of
    /// rights. Doing the escalation inside the per-command helper prompts
    /// once per networksetup call (18 password dialogs for one enable).
    fn run_batch(cmds: &[Vec<String>]) -> Result<(), String> {
        let mut errors = Vec::new();
        let mut denied: Option<String> = None;
        for args in cmds {
            match run_plain(args) {
                Ok(()) => {}
                Err(e) if privilege_denied(&e) => denied = Some(e),
                Err(e) => errors.push(format!("{}: {e}", args.join(" "))),
            }
        }
        if let Some(e) = denied {
            return run_admin(cmds).map_err(|e2| format!("{e}；管理员回退失败: {e2}"));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Replay the batch in a single `do shell script … with administrator
    /// privileges` run: one password prompt for the whole batch.
    fn run_admin(cmds: &[Vec<String>]) -> Result<(), String> {
        let body = cmds
            .iter()
            .map(|args| {
                std::iter::once(NETWORKSETUP)
                    .chain(args.iter().map(String::as_str))
                    .map(sh_quote)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("; ");
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            body.replace('"', "\\\"")
        );
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("运行 osascript: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    /// Proxy sub-commands driven on every network service.
    const KINDS: [&str; 3] = ["web", "secureweb", "socksfirewall"];

    /// Parse `networksetup -listallnetworkservices` stdout. It prints an
    /// explanatory banner ("An asterisk (*) denotes …") *before* the list,
    /// and marks disabled services with a leading asterisk; neither is a
    /// service. Feeding the banner back into networksetup fails with
    /// "The parameters were not valid", which used to trigger the
    /// administrator fallback on every proxy toggle.
    fn parse_services(stdout: &str) -> Vec<String> {
        stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('*') && !l.starts_with("An asterisk"))
            .map(str::to_string)
            .collect()
    }

    /// Network services that can carry a proxy (skips disabled/`*` entries).
    fn services() -> Result<Vec<String>, String> {
        let out = std::process::Command::new(NETWORKSETUP)
            .arg("-listallnetworkservices")
            .output()
            .map_err(|e| format!("运行 networksetup -listallnetworkservices: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        let list = parse_services(&String::from_utf8_lossy(&out.stdout));
        if list.is_empty() {
            return Err("未找到可用的网络服务".into());
        }
        Ok(list)
    }

    /// Mutations that route every service's HTTP/HTTPS/SOCKS traffic through
    /// `127.0.0.1:port`.
    fn enable_cmds(list: &[String], port: &str) -> Vec<Vec<String>> {
        let mut cmds = Vec::new();
        for svc in list {
            for kind in KINDS {
                cmds.push(vec![
                    format!("-set{kind}proxy"),
                    svc.clone(),
                    HOST.to_string(),
                    port.to_string(),
                ]);
                cmds.push(vec![
                    format!("-set{kind}proxystate"),
                    svc.clone(),
                    "on".into(),
                ]);
            }
        }
        cmds
    }

    /// Mutations that turn every service's proxy state off.
    fn disable_cmds(list: &[String]) -> Vec<Vec<String>> {
        let mut cmds = Vec::new();
        for svc in list {
            for kind in KINDS {
                cmds.push(vec![
                    format!("-set{kind}proxystate"),
                    svc.clone(),
                    "off".into(),
                ]);
            }
        }
        cmds
    }

    /// Read back one service's proxy state: `(enabled, server, port)`.
    /// `None` when networksetup cannot report it.
    fn proxy_state(svc: &str, kind: &str) -> Option<(bool, String, String)> {
        let out = std::process::Command::new(NETWORKSETUP)
            .args([&format!("-get{kind}proxy"), svc])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let (mut enabled, mut server, mut port) = (false, String::new(), String::new());
        for line in text.lines() {
            let line = line.trim();
            if let Some(v) = line.strip_prefix("Enabled:") {
                enabled = v.trim() == "Yes";
            } else if let Some(v) = line.strip_prefix("Server:") {
                server = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Port:") {
                port = v.trim().to_string();
            }
        }
        Some((enabled, server, port))
    }

    /// Read back one service's web-proxy state.
    fn web_proxy(svc: &str) -> Option<(bool, String, String)> {
        proxy_state(svc, "web")
    }

    /// How many services route web traffic through `host:port` right now.
    fn web_proxy_at(list: &[String], host: &str, port: &str) -> usize {
        list.iter()
            .filter(|svc| web_proxy(svc).is_some_and(|(on, h, p)| on && h == host && p == port))
            .count()
    }

    pub fn enable(port: u16) -> Result<(), String> {
        let list = services()?;
        let port = port.to_string();
        let applied = run_batch(&enable_cmds(&list, &port));
        // networksetup exits 0 for services it silently ignores, so the
        // read-back decides, not the exit codes: at least one service must
        // actually route through us. Anything else means that service
        // cannot carry a proxy (Thunderbolt Bridge, serial ports) — the
        // ones that applied are the ones that matter.
        if web_proxy_at(&list, HOST, &port) == 0 {
            return Err(match applied {
                Err(e) => e,
                Ok(()) => "networksetup 未生效：没有网络服务接受代理设置".into(),
            });
        }
        if let Err(e) = applied {
            eprintln!("system proxy（部分服务失败）: {e}");
        }
        Ok(())
    }

    pub fn disable() -> Result<(), String> {
        let list = services()?;
        let applied = run_batch(&disable_cmds(&list));
        let still_on = list
            .iter()
            .filter(|svc| web_proxy(svc).is_some_and(|(on, ..)| on))
            .count();
        if still_on > 0 {
            return Err(match applied {
                Err(e) => e,
                Ok(()) => format!("仍有 {still_on} 个网络服务的代理处于开启状态"),
            });
        }
        if let Err(e) = applied {
            eprintln!("system proxy（部分服务失败）: {e}");
        }
        Ok(())
    }

    /// True when any network service still routes web traffic through
    /// 127.0.0.1:port with the proxy enabled (a crash could not run
    /// disable). Startup restores it so traffic does not hit a dead
    /// orphaned core after reaping. Other clients' proxies on other ports
    /// are none of our business.
    pub fn leftover_at(port: u16) -> bool {
        let Ok(list) = services() else {
            return false;
        };
        web_proxy_at(&list, HOST, &port.to_string()) > 0
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Real `networksetup -listallnetworkservices` stdout: the banner and
        /// disabled (asterisk-prefixed) services are not services. Feeding
        /// the banner back to networksetup fails with "The parameters were
        /// not valid" (verified on macOS 26.7), which used to escalate to an
        /// administrator prompt on every proxy toggle.
        #[test]
        fn banner_and_disabled_services_are_skipped() {
            let stdout = "An asterisk (*) denotes that a network service is disabled.\n\
                          Built-in Serial Port (0)\n\
                          *Thunderbolt Bridge\n\
                          Ethernet\n\
                          Wi-Fi\n";
            assert_eq!(
                parse_services(stdout),
                vec!["Built-in Serial Port (0)", "Ethernet", "Wi-Fi"]
            );
        }

        /// Only privilege refusals may raise a password dialog; parameter
        /// errors (the real text for the banner bug) must not.
        #[test]
        fn only_privilege_failures_escalate() {
            assert!(!privilege_denied(
                "** Error: The parameters were not valid."
            ));
            assert!(privilege_denied(
                "** Error: You must be running as root to change the proxy settings."
            ));
            assert!(privilege_denied("Permission denied"));
        }

        #[test]
        fn shell_words_are_quoted() {
            assert_eq!(sh_quote("Wi-Fi"), "'Wi-Fi'");
            assert_eq!(
                sh_quote("Built-in Serial Port (0)"),
                "'Built-in Serial Port (0)'"
            );
            assert_eq!(sh_quote("it's"), r"'it'\''s'");
        }

        #[test]
        fn enable_and_disable_command_sets() {
            let list = vec!["Wi-Fi".to_string()];
            let on = enable_cmds(&list, "2080");
            assert_eq!(on.len(), 6, "set + state for each of the 3 kinds");
            assert_eq!(on[0], vec!["-setwebproxy", "Wi-Fi", "127.0.0.1", "2080"]);
            assert_eq!(on[1], vec!["-setwebproxystate", "Wi-Fi", "on"]);
            assert!(on.iter().any(|c| c[0] == "-setsocksfirewallproxy"));
            let off = disable_cmds(&list);
            assert_eq!(off.len(), 3);
            assert!(off.iter().all(|c| c[2] == "off"));
        }

        /// The real service list on this machine must survive the banner
        /// filter (read-only; no proxy state is touched).
        #[test]
        fn real_service_list_has_no_banner() {
            let list = services().expect("networksetup -listallnetworkservices");
            assert!(!list.is_empty());
            assert!(
                !list.iter().any(|s| s.starts_with("An asterisk")),
                "banner leaked into the service list: {list:?}"
            );
            assert!(!list.iter().any(|s| s.starts_with('*')));
        }

        /// Real-machine round trip of the whole macOS proxy path: enable,
        /// read back, disable, then put every service back as it was.
        /// Opt-in because it mutates the machine's proxy settings:
        /// `cargo test --lib -- --ignored system_proxy_round_trip`.
        #[test]
        #[ignore = "mutates the machine's system proxy settings (restored afterwards)"]
        fn system_proxy_round_trip() {
            const PORT: u16 = 65001;
            let list = services().expect("service list");
            // Snapshot before touching anything, restored at the end.
            let snapshot: Vec<(String, Vec<(&str, (bool, String, String))>)> = list
                .iter()
                .map(|svc| {
                    (
                        svc.clone(),
                        KINDS
                            .iter()
                            .map(|k| {
                                (
                                    *k,
                                    proxy_state(svc, k)
                                        .unwrap_or((false, String::new(), String::new())),
                                )
                            })
                            .collect(),
                    )
                })
                .collect();

            fn restore(snapshot: &[(String, Vec<(&str, (bool, String, String))>)]) {
                for (svc, kinds) in snapshot {
                    for (kind, (on, server, port)) in kinds {
                        if !server.is_empty() {
                            let _ = run_plain(&[
                                format!("-set{kind}proxy"),
                                svc.clone(),
                                server.clone(),
                                port.clone(),
                            ]);
                        }
                        let _ = run_plain(&[
                            format!("-set{kind}proxystate"),
                            svc.clone(),
                            if *on { "on".into() } else { "off".into() },
                        ]);
                    }
                }
            }

            let result = std::panic::catch_unwind(|| {
                enable(PORT).expect("enable");
                assert!(
                    web_proxy_at(&list, HOST, &PORT.to_string()) > 0,
                    "enable did not take effect on any service"
                );
                assert!(leftover_at(PORT), "leftover_at must see our own route");
                disable().expect("disable");
                assert_eq!(
                    list.iter()
                        .filter(|svc| web_proxy(svc).is_some_and(|(on, ..)| on))
                        .count(),
                    0,
                    "disable left a proxy enabled"
                );
                assert!(!leftover_at(PORT));
            });
            restore(&snapshot);
            if let Err(panic) = result {
                std::panic::resume_unwind(panic);
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux_impl::{disable, enable, leftover_at};
#[cfg(target_os = "windows")]
pub use win_impl::{disable, enable, leftover_at};
#[cfg(target_os = "macos")]
pub use mac_impl::{disable, enable, leftover_at};

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn enable(_port: u16) -> Result<(), String> {
    Err("system proxy: not implemented on this platform yet".into())
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn disable() -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn leftover_at(_port: u16) -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn api_shape() {
        // Per-platform bodies are compiled & exercised on their own OS; this
        // keeps the module compiling on every target.
        let _ = std::any::type_name::<(fn(u16) -> Result<(), String>, fn() -> Result<(), String>)>();
    }
}
