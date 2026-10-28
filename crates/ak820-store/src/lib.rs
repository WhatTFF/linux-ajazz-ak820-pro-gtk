//! Persistent state shared by the GUI, the CLI and the autosync daemon.
//!
//! History is stored as raw `0x50` payloads in `$XDG_DATA_HOME/ak820/history`,
//! byte-identical to the upstream Python CLI, so all tools can read each other's
//! uploads. Settings live next to it in `settings.json`.

#![forbid(unsafe_code)]

pub mod history;
pub mod settings;

pub use history::Entry;

/// `$XDG_DATA_HOME/ak820`.
pub fn data_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".local/share")
        })
        .join("ak820")
}
