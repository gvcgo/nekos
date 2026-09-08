//! Autostart platform abstraction.
//!
//! - Linux: XDG `.desktop` entry under `$XDG_CONFIG_HOME/autostart`.
//! - Windows: `HKCU\...\Run` value pointing at the executable.
//! - macOS: a `LaunchAgent` plist loaded with `launchctl`.

#[cfg(target_os = "linux")]
mod linux_impl {
    use std::path::PathBuf;

    const ENTRY_NAME: &str = "nekos.desktop";

    fn config_home() -> PathBuf {
        if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
            if !dir.trim().is_empty() {
                return PathBuf::from(dir);
            }
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
    }

    /// Where the autostart entry lives.
    pub fn desktop_path() -> PathBuf {
        config_home().join("autostart").join(ENTRY_NAME)
    }

    fn exec_escape(s: &str) -> String {
        // .desktop Exec field: quote arguments containing spaces.
        if s.contains(char::is_whitespace) {
            format!("\"{}\"", s.replace('"', "\\\""))
        } else {
            s.to_string()
        }
    }

    /// Write (or refresh) the autostart entry for the running executable.
    pub fn enable() -> Result<(), String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("current exe: {e}"))?;
        let exe = exe.to_string_lossy().into_owned();
        let path = desktop_path();
        let dir = path.parent().unwrap_or(path.as_path());
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let content = format!(
            "[Desktop Entry]\nType=Application\nVersion=1.0\nName=nekos\nComment=nekos proxy client\nExec={}\nX-GNOME-Autostart-enabled=true\n",
            exec_escape(&exe)
        );
        std::fs::write(&path, content)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(())
    }

    /// Remove the autostart entry (no-op when absent).
    pub fn disable() -> Result<(), String> {
        let path = desktop_path();
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("remove {}: {e}", path.display())),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static SEQ: AtomicUsize = AtomicUsize::new(0);

        struct XdgGuard(PathBuf);

        impl Drop for XdgGuard {
            fn drop(&mut self) {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
        }

        fn with_temp_xdg() -> (XdgGuard, PathBuf) {
            let dir = std::env::temp_dir().join(format!(
                "nekos-xdg-test-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::SeqCst)
            ));
            std::env::set_var("XDG_CONFIG_HOME", &dir);
            (XdgGuard(dir.clone()), dir)
        }

        #[test]
        fn enable_writes_entry_disable_removes() {
            let (_guard, dir) = with_temp_xdg();
            enable().expect("enable");
            let path = desktop_path();
            assert!(path.starts_with(&dir));
            let content = std::fs::read_to_string(&path).expect("read entry");
            assert!(content.contains("[Desktop Entry]"));
            assert!(content.contains("Type=Application"));
            let exe = std::env::current_exe().unwrap();
            let escaped = if exe.to_string_lossy().contains(char::is_whitespace) {
                format!("\"{}\"", exe.display())
            } else {
                exe.to_string_lossy().into_owned()
            };
            assert!(content.contains(&format!("Exec={escaped}")));
            // refresh (idempotent)
            enable().expect("re-enable");
            disable().expect("disable");
            assert!(!path.exists());
            // disabling again is a no-op
            disable().expect("disable again");
        }
    }
}

#[cfg(target_os = "windows")]
mod win_impl {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "nekos";

    /// Register (or refresh) the autostart entry in the current user's Run key.
    pub fn enable() -> Result<(), String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("current exe: {e}"))?
            .to_string_lossy()
            .into_owned();
        // Quote so paths with spaces survive Explorer's command-line parse.
        // winreg writes REG_SZ for &str values.
        let cmd = format!("\"{exe}\"");
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _disp) = hkcu
            .create_subkey(RUN_KEY)
            .map_err(|e| format!("打开注册表 {RUN_KEY}: {e}"))?;
        key.set_value(VALUE_NAME, &cmd.as_str())
            .map_err(|e| format!("写入 Run 值: {e}"))?;
        Ok(())
    }

    /// Remove the autostart entry (no-op when absent).
    pub fn disable() -> Result<(), String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _disp) = hkcu
            .create_subkey(RUN_KEY)
            .map_err(|e| format!("打开注册表 {RUN_KEY}: {e}"))?;
        match key.delete_value(VALUE_NAME) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删除 Run 值: {e}")),
        }
    }
}

#[cfg(target_os = "macos")]
mod mac_impl {
    use std::path::PathBuf;

    const LABEL: &str = "app.nekos.desktop";
    const FILE_NAME: &str = "app.nekos.desktop.plist";

    fn agents_dir() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library/LaunchAgents")
    }

    fn plist_path() -> PathBuf {
        agents_dir().join(FILE_NAME)
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn current_uid() -> Option<String> {
        let out = std::process::Command::new("id").arg("-u").output().ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    }

    /// Load the agent into the current GUI session (best effort: the plist is
    /// enough for next login; this makes the toggle effective immediately).
    fn launchctl_load() {
        let Some(uid) = current_uid() else { return };
        let plist = plist_path();
        let _ = std::process::Command::new("launchctl")
            .args(["bootstrap", &format!("gui/{uid}"), &plist.to_string_lossy()])
            .output();
    }

    fn launchctl_unload() {
        let Some(uid) = current_uid() else { return };
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{LABEL}")])
            .output();
    }

    /// Write (or refresh) the LaunchAgent and load it for the current session.
    pub fn enable() -> Result<(), String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("current exe: {e}"))?
            .to_string_lossy()
            .into_owned();
        let path = plist_path();
        let dir = path.parent().unwrap_or(path.as_path());
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let content = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             \t<key>Label</key>\n\t<string>{LABEL}</string>\n\
             \t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\t</array>\n\
             \t<key>RunAtLoad</key>\n\t<true/>\n\
             </dict>\n</plist>\n",
            xml_escape(&exe)
        );
        std::fs::write(&path, content)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        launchctl_load();
        Ok(())
    }

    /// Remove the LaunchAgent (no-op when absent).
    pub fn disable() -> Result<(), String> {
        launchctl_unload();
        let path = plist_path();
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("remove {}: {e}", path.display())),
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux_impl::{disable, enable};
#[cfg(target_os = "windows")]
pub use win_impl::{disable, enable};
#[cfg(target_os = "macos")]
pub use mac_impl::{disable, enable};
