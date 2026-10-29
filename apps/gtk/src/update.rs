//! Update download/install, uninstall, restart and package detection.
//!
//! Package installs go through `pkexec`, so the desktop shows its own password
//! prompt. Every operation also returns a command the user can run by hand.

use std::path::{Path, PathBuf};

use crate::net;

/// `"appimage"`, `"rpm"`, `"deb"` or `"source"`, guessed from this binary.
pub fn package_kind() -> &'static str {
    if std::env::var_os("APPIMAGE").is_some() {
        return "appimage";
    }
    let Ok(exe) = std::env::current_exe() else {
        return "source";
    };
    let exe = exe.to_string_lossy().to_string();
    let owned = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .arg(&exe)
            .output()
            .is_ok_and(|out| out.status.success())
    };
    if owned("rpm", &["-qf"]) {
        "rpm"
    } else if owned("dpkg", &["-S"]) {
        "deb"
    } else {
        "source"
    }
}

/// The package extension this build can install.
pub fn asset_extension() -> &'static str {
    match package_kind() {
        "appimage" => ".AppImage",
        "deb" => ".deb",
        _ => ".rpm",
    }
}

/// `~/Downloads`, or the data directory when there is none.
pub fn download_dir() -> PathBuf {
    std::env::var_os("XDG_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Downloads")))
        .filter(|p| p.is_dir())
        .unwrap_or_else(ak820_store::data_dir)
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '_',
            c => c,
        })
        .take(180)
        .collect();
    if cleaned.is_empty() {
        "ak820-gtk".into()
    } else {
        cleaned
    }
}

/// The command a user can paste instead of using the password prompt.
pub fn install_command(kind: &str, path: &Path) -> String {
    let path = path.display();
    match kind {
        "deb" => format!("sudo apt install -y \"{path}\""),
        _ => format!("sudo dnf install -y \"{path}\""),
    }
}

/// The command that removes this build.
#[allow(dead_code)]
pub fn uninstall_command() -> String {
    match package_kind() {
        "deb" => "sudo apt remove -y ak820-gtk".into(),
        "rpm" => "sudo dnf remove -y ak820-gtk".into(),
        "appimage" => std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .map(|p| format!("rm -f \"{}\"", p.display()))
            .unwrap_or_else(|| "rm -f ./ak820-gtk_*.AppImage".into()),
        _ => "\
# Manual removal:
#   rm /usr/local/bin/ak820-gtk /usr/local/bin/ak820ctl
#   sudo rm -f /usr/lib/udev/rules.d/70-ak820.rules
#   sudo rm -f /usr/lib/systemd/user/ak820-clock.service
#   sudo rm -f /usr/lib/systemd/user/ak820-clock.timer
#   sudo rm -f /usr/lib/systemd/user/ak820-connect.service
#   sudo rm -f /usr/lib/systemd/user/ak820-resume.service
#   sudo rm -f /usr/share/applications/ak820-gtk.desktop
#   sudo rm -f /usr/share/icons/hicolor/256x256/apps/ak820-gtk.png
#   rm -rf ~/.local/share/ak820"
            .into(),
    }
}

/// Download a release asset into the download directory.
pub fn download(url: &str, name: &str) -> Result<PathBuf, String> {
    let dir = download_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let target = dir.join(sanitize_file_name(name));
    let tmp = target.with_extension("part");
    net::download_release(url, &tmp)?;
    std::fs::rename(&tmp, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(target)
}

/// Install a package only if it came from our download directory.
fn is_installable(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if ext != "rpm" && ext != "deb" {
        return false;
    }
    let (Ok(file), Ok(dir)) = (path.canonicalize(), download_dir().canonicalize()) else {
        return false;
    };
    file.starts_with(dir) && file.is_file()
}

/// Install a downloaded RPM/DEB through polkit.
pub fn install(path: &Path) -> Result<String, String> {
    if !is_installable(path) {
        return Err(format!("not a downloaded .rpm or .deb: {}", path.display()));
    }
    let kind = package_kind();
    let mut cmd = std::process::Command::new("pkexec");
    match kind {
        "deb" => cmd.args(["apt-get", "install", "-y"]),
        _ => cmd.args(["dnf", "install", "-y"]),
    };
    let out = cmd.arg(path).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "pkexec is not installed. Run: {}",
                install_command(kind, path)
            )
        } else {
            format!("pkexec: {e}")
        }
    })?;
    if out.status.success() {
        let _ = std::fs::remove_file(path);
        Ok("Installed. Restart the app to use the new version.".into())
    } else if matches!(out.status.code(), Some(126) | Some(127)) {
        Err(format!(
            "The password prompt was dismissed, so nothing was installed. Run: {}",
            install_command(kind, path)
        ))
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail = err.lines().rev().take(4).collect::<Vec<_>>().join("\n");
        Err(format!(
            "The installer failed ({}). {}",
            out.status,
            tail.trim()
        ))
    }
}

/// Replace this AppImage in place.
pub fn replace_appimage(url: &str) -> Result<String, String> {
    let current = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .ok_or("this is not an AppImage build")?;
    let tmp = current.with_extension("AppImage.new");
    net::download_release(url, &tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &current).map_err(|e| format!("{}: {e}", current.display()))?;
    Ok("AppImage replaced. Restart to use the new version.".into())
}

/// Remove this app with the system package manager, or delete the AppImage.
#[allow(dead_code)]
pub fn uninstall() -> Result<String, String> {
    match package_kind() {
        "deb" | "rpm" => {
            let kind = package_kind();
            let mut cmd = std::process::Command::new("pkexec");
            match kind {
                "deb" => cmd.args(["apt-get", "remove", "-y", "ak820-gtk"]),
                _ => cmd.args(["dnf", "remove", "-y", "ak820-gtk"]),
            };
            let out = cmd.output().map_err(|e| format!("pkexec: {e}"))?;
            if out.status.success() {
                Ok("Uninstalled. The app will close now.".into())
            } else {
                Err(format!(
                    "Uninstall failed ({}). Run: {}",
                    out.status,
                    uninstall_command()
                ))
            }
        }
        "appimage" => {
            let current = std::env::var_os("APPIMAGE")
                .map(PathBuf::from)
                .ok_or("this is not an AppImage build")?;
            std::fs::remove_file(&current)
                .map_err(|e| format!("{}: {e}", current.display()))?;
            Ok("AppImage deleted. Quit the app now.".into())
        }
        _ => Err(
            "This build was not installed from a package. Use the command below to remove it manually."
                .into(),
        ),
    }
}

/// Re-exec this binary. Returns only on failure.
pub fn restart() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("restart: {e}"))
}

/// Only hand well-formed web URLs to `xdg-open`.
pub fn open_url(url: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| format!("refusing to open {url}"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty()
        || rest.len() > 2048
        || url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\\')
    {
        return Err(format!("refusing to open {url}"));
    }
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("xdg-open: {e}"))
}

/// Semver-ish comparison used for release tags (`v0.2.1-beta`).
///
/// A release beats a prerelease of the same version, and numeric identifiers
/// compare numerically (`0.2.10` > `0.2.9`).
pub fn newer_version(a: &str, b: &str) -> std::cmp::Ordering {
    fn parse(tag: &str) -> (Vec<u64>, Vec<String>) {
        let clean = tag.trim().trim_start_matches(['v', 'V']);
        let clean = clean.split('+').next().unwrap_or(clean);
        let (core, pre) = match clean.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (clean, None),
        };
        let core = core
            .split('.')
            .map(|n| n.parse::<u64>().unwrap_or(0))
            .collect();
        let pre = pre
            .map(|p| p.split('.').map(str::to_string).collect())
            .unwrap_or_default();
        (core, pre)
    }
    let (ac, ap) = parse(a);
    let (bc, bp) = parse(b);
    for i in 0..ac.len().max(bc.len()) {
        let x = ac.get(i).copied().unwrap_or(0);
        let y = bc.get(i).copied().unwrap_or(0);
        if x != y {
            return x.cmp(&y);
        }
    }
    if ap.is_empty() && bp.is_empty() {
        return std::cmp::Ordering::Equal;
    }
    if ap.is_empty() {
        return std::cmp::Ordering::Greater;
    }
    if bp.is_empty() {
        return std::cmp::Ordering::Less;
    }
    for i in 0..ap.len().max(bp.len()) {
        let x = ap.get(i);
        let y = bp.get(i);
        let ord = match (x, y) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => {
                let xn = x.parse::<u64>();
                let yn = y.parse::<u64>();
                match (xn, yn) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                    (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                }
            }
        };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    std::cmp::Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn version_ordering() {
        assert_eq!(newer_version("v0.2.1", "v0.2.0"), Ordering::Greater);
        assert_eq!(newer_version("v0.2.10", "v0.2.9"), Ordering::Greater);
        assert_eq!(newer_version("v0.2.1", "v0.2.1-beta"), Ordering::Greater);
        assert_eq!(newer_version("v0.2.1-beta", "v0.2.1-beta"), Ordering::Equal);
        assert_eq!(
            newer_version("v0.2.1-beta", "v0.2.0-beta"),
            Ordering::Greater
        );
        assert_eq!(newer_version("v0.2.1-alpha", "v0.2.1-beta"), Ordering::Less);
        assert_eq!(
            newer_version("v0.2.1-beta.2", "v0.2.1-beta"),
            Ordering::Greater
        );
    }

    #[test]
    fn uninstall_commands_are_shell_safe() {
        assert!(uninstall_command().contains('\n') || uninstall_command().starts_with("sudo"));
    }

    #[test]
    fn sanitises_asset_names() {
        assert_eq!(sanitize_file_name("../../x.rpm"), ".._.._x.rpm");
        assert_eq!(sanitize_file_name(""), "ak820-gtk");
    }
}
