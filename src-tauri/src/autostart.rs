//! XDG autostart for Linux: a `.desktop` entry under
//! `$XDG_CONFIG_HOME/autostart` (default `~/.config/autostart`) pointing at
//! the running executable. Windows/macOS equivalents belong to P4.

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
