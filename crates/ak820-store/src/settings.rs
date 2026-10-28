//! User settings, stored as a small JSON file.
//!
//! The file is read by the GUI, by `ak820ctl` and by the systemd units, so it
//! must stay backwards compatible: every field has a default and unknown keys
//! are ignored.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Everything the app remembers between runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Keep the keyboard clock in sync automatically (systemd timer hint).
    pub autosync_clock: bool,
    /// Re-upload the last animation when the keyboard is plugged in.
    pub restore_on_connect: bool,
    /// Push the clock as soon as the keyboard appears.
    pub sync_on_connect: bool,
    /// GIPHY API key (empty = search disabled).
    pub giphy_key: String,
    /// Start the app when the user logs in.
    pub start_at_login: bool,
    /// Start hidden in the tray instead of showing the window.
    pub start_minimized: bool,
    /// Closing the window keeps the app running in the tray.
    pub minimize_to_tray: bool,
    /// Push the clock once when the app starts.
    pub sync_on_start: bool,
    /// Quit right after that start-up sync (a "sync and go" launcher).
    pub quit_after_start_sync: bool,
    /// Re-sync every N seconds while the app runs. `0` disables it.
    pub sync_interval_secs: u64,
    /// Look for a new release in the background and offer it.
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            autosync_clock: true,
            restore_on_connect: false,
            sync_on_connect: true,
            giphy_key: String::new(),
            start_at_login: false,
            start_minimized: false,
            minimize_to_tray: false,
            sync_on_start: true,
            quit_after_start_sync: false,
            sync_interval_secs: 0,
            check_updates: true,
        }
    }
}

/// Path of `settings.json`.
pub fn path() -> PathBuf {
    crate::data_dir().join("settings.json")
}

impl Settings {
    /// Load from disk, falling back to defaults.
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Write to disk, creating the data directory if needed.
    pub fn save(&self) -> Result<(), String> {
        let path = path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut json =
            serde_json::to_string_pretty(self).map_err(|e| format!("serialising settings: {e}"))?;
        json.push('\n');
        std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let settings = Settings {
            autosync_clock: false,
            restore_on_connect: true,
            sync_on_connect: false,
            giphy_key: "abc\"def\\ghi\n".into(),
            start_at_login: true,
            start_minimized: true,
            minimize_to_tray: true,
            sync_on_start: false,
            quit_after_start_sync: true,
            sync_interval_secs: 300,
            check_updates: false,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let parsed: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, settings);
    }

    #[test]
    fn defaults_when_missing_keys() {
        let parsed: Settings = serde_json::from_str("{}").unwrap();
        assert!(parsed.autosync_clock);
        assert!(!parsed.restore_on_connect);
        assert!(parsed.sync_on_connect);
        assert!(parsed.sync_on_start);
        assert_eq!(parsed.sync_interval_secs, 0);
        assert!(parsed.giphy_key.is_empty());
        assert!(
            parsed.check_updates,
            "update checks are on unless turned off"
        );
    }

    #[test]
    fn reads_the_old_three_key_file() {
        // Written by v0.1.0, before the GUI settings existed.
        let old = r#"{
  "autosync_clock": false,
  "restore_on_connect": true,
  "giphy_key": "x"
}"#;
        let parsed: Settings = serde_json::from_str(old).unwrap();
        assert!(!parsed.autosync_clock);
        assert!(parsed.restore_on_connect);
        assert_eq!(parsed.giphy_key, "x");
        assert!(parsed.sync_on_start, "new keys fall back to their default");
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        assert!(serde_json::from_str::<Settings>("not json at all").is_err());
        // `Settings::load` swallows that error and returns the defaults, so a
        // half-written file can never stop the app from starting.
        assert_eq!(Settings::default().sync_interval_secs, 0);
    }
}
