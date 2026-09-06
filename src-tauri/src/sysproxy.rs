//! System-proxy platform abstraction. Linux (GNOME) uses GSettings; other
//! platforms report not-implemented until their adapters land (P4).

#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
pub fn disable() -> Result<(), String> {
    gsettings(&["set", "org.gnome.system.proxy", "mode", "none"])
}

#[cfg(not(target_os = "linux"))]
pub fn enable(_port: u16) -> Result<(), String> {
    Err("system proxy: not implemented on this platform yet".into())
}

#[cfg(not(target_os = "linux"))]
pub fn disable() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn linux_compiles() {
        // module compiles on all targets; behavior tested in E2E on Linux
    }
}
