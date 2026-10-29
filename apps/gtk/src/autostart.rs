//! Autostart entry, mirroring the Tauri app's `~/.config/autostart` file.

use std::path::PathBuf;

fn base_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
}

fn path() -> PathBuf {
    base_dir().join("autostart").join("ak820-gtk.desktop")
}

/// Create or remove the XDG autostart entry.
pub fn set(enabled: bool) -> Result<(), String> {
    set_in(&path(), enabled)
}

fn set_in(path: &std::path::Path, enabled: bool) -> Result<(), String> {
    if !enabled {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        };
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ak820-gtk"));
    let exec = quote_exec(&exe);
    let entry = format!(
        "\
[Desktop Entry]
Type=Application
Name=AK820 Screen Studio
Comment=Keyboard screen studio and clock sync
Exec={exec} --tray
Icon=ak820-gtk
Terminal=false
X-GNOME-Autostart-enabled=true
"
    );
    std::fs::write(path, entry).map_err(|e| format!("{}: {e}", path.display()))
}

/// Quote a program path for a Desktop Entry `Exec=` line.
fn quote_exec(path: &std::path::Path) -> String {
    let raw = path.to_string_lossy();
    let needs_quote = raw
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '`'));
    if !needs_quote {
        return raw.into_owned();
    }
    let escaped = raw.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_removes_entry() {
        let dir = std::env::temp_dir().join(format!("ak820-autostart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("ak820-gtk.desktop");
        set_in(&path, true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Exec="));
        assert!(text.contains("--tray"));
        set_in(&path, false).unwrap();
        assert!(!path.exists());
        set_in(&path, false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
