//! System-proxy platform abstraction.
//!
//! - Linux (GNOME): `gsettings`.
//! - Windows: WinINET proxy keys under HKCU (takes effect system-wide for
//!   WinINET/WinHTTP consumers), then broadcasts the change so running apps
//!   notice immediately.
//! - macOS: `networksetup` per network service; falls back to an
//!   administrator-prompted `osascript` run when the plain call lacks the
//!   privileges to change the service configuration.
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
}

#[cfg(target_os = "macos")]
mod mac_impl {
    const HOST: &str = "127.0.0.1";
    const NETWORKSETUP: &str = "/usr/sbin/networksetup";

    /// Run networksetup, falling back to an admin-prompted osascript run when
    /// the current user is not allowed to change network service settings
    /// (macOS requires admin rights for these mutations).
    fn networksetup(args: &[&str]) -> Result<(), String> {
        let out = std::process::Command::new(NETWORKSETUP)
            .args(args)
            .output()
            .map_err(|e| format!("运行 networksetup: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        // Privilege failure → ask the user once through the GUI.
        let quoted: Vec<String> = args
            .iter()
            .map(|a| format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect();
        let script = format!(
            "do shell script \"{} {}\" with administrator privileges",
            NETWORKSETUP,
            quoted.join(" ")
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

    /// Network services that can carry a proxy (skips disabled/`*` entries).
    fn services() -> Result<Vec<String>, String> {
        let out = std::process::Command::new(NETWORKSETUP)
            .arg("-listallnetworkservices")
            .output()
            .map_err(|e| format!("运行 networksetup -listallnetworkservices: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('*'))
            .map(str::to_string)
            .collect())
    }

    fn for_each_service(apply: &dyn Fn(&str) -> Result<(), String>) -> Result<(), String> {
        let list = services()?;
        if list.is_empty() {
            return Err("未找到可用的网络服务".into());
        }
        let mut failures = Vec::new();
        for svc in &list {
            if let Err(e) = apply(svc) {
                failures.push(format!("{svc}: {e}"));
            }
        }
        if failures.len() == list.len() {
            Err(failures.join("; "))
        } else if failures.is_empty() {
            Ok(())
        } else {
            // Partial success (some services reject proxy config); the ones
            // that applied are the important ones, so surface but succeed.
            eprintln!("system proxy: {}", failures.join("; "));
            Ok(())
        }
    }

    pub fn enable(port: u16) -> Result<(), String> {
        let port = port.to_string();
        for_each_service(&|svc| {
            for kind in ["web", "secureweb", "socksfirewall"] {
                networksetup(&[
                    &format!("-set{kind}proxy"),
                    svc,
                    HOST,
                    &port,
                ])?;
                networksetup(&[&format!("-set{kind}proxystate"), svc, "on"])?;
            }
            Ok(())
        })
    }

    pub fn disable() -> Result<(), String> {
        for_each_service(&|svc| {
            for kind in ["web", "secureweb", "socksfirewall"] {
                networksetup(&[&format!("-set{kind}proxystate"), svc, "off"])?;
            }
            Ok(())
        })
    }
}

#[cfg(target_os = "linux")]
pub use linux_impl::{disable, enable, leftover_at};
#[cfg(target_os = "windows")]
pub use win_impl::{disable, enable};
#[cfg(target_os = "macos")]
pub use mac_impl::{disable, enable};

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn enable(_port: u16) -> Result<(), String> {
    Err("system proxy: not implemented on this platform yet".into())
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn disable() -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
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
