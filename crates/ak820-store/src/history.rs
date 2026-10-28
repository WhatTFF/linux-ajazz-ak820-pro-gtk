//! On-disk upload history, byte-compatible with `ak820.py` (`~/.local/share/ak820/history`).

use std::path::{Path, PathBuf};

/// Keep at most this many uploads (upstream CLI uses 50).
pub const MAX_ENTRIES: usize = 50;

/// One stored upload.
#[derive(Clone, Debug)]
pub struct Entry {
    /// 1-based index, newest first.
    pub index: usize,
    /// Absolute path of the `.bin` payload.
    pub path: PathBuf,
    /// Human name (file stem after the timestamp).
    pub name: String,
    /// Payload size in bytes.
    pub bytes: u64,
}

/// `$XDG_DATA_HOME/ak820/history`.
pub fn dir() -> PathBuf {
    crate::data_dir().join("history")
}

/// All stored uploads, newest first.
pub fn list() -> Vec<Entry> {
    list_in(&dir())
}

fn list_in(dir: &Path) -> Vec<Entry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "bin"))
        .collect();
    files.sort();
    files.reverse();

    files
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let name = stem
                .split_once('_')
                .map(|(_, n)| n)
                .unwrap_or(stem)
                .to_string();
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            Entry {
                index: i + 1,
                path,
                name,
                bytes,
            }
        })
        .collect()
}

/// Store a payload, prune old entries, and return the path written.
pub fn save(payload: &[u8], name: &str) -> Result<PathBuf, String> {
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("{stamp}_{safe}.bin"));
    std::fs::write(&path, payload).map_err(|e| format!("{}: {e}", path.display()))?;

    for old in list().into_iter().skip(MAX_ENTRIES) {
        let _ = std::fs::remove_file(old.path);
    }
    Ok(path)
}

/// Load entry `index` (1 = newest).
pub fn load(index: usize) -> Result<Vec<u8>, String> {
    if index == 0 {
        return Err("history entries are numbered from 1".into());
    }
    let entry = list()
        .into_iter()
        .find(|e| e.index == index)
        .ok_or_else(|| format!("no history entry {index} ({} stored)", list().len()))?;
    std::fs::read(&entry.path).map_err(|e| format!("{}: {e}", entry.path.display()))
}

/// Delete every stored upload; returns how many were removed.
pub fn clear() -> Result<usize, String> {
    let entries = list();
    for entry in &entries {
        std::fs::remove_file(&entry.path).map_err(|e| format!("{}: {e}", entry.path.display()))?;
    }
    Ok(entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ak820-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn listing_is_newest_first_and_numbered() {
        let dir = tempdir("history");
        std::fs::write(dir.join("20260101-000000_old.bin"), [1u8]).unwrap();
        std::fs::write(dir.join("20260102-000000_new.bin"), [2u8, 3]).unwrap();
        std::fs::write(dir.join("notes.txt"), [0u8]).unwrap();

        let entries = list_in(&dir);
        assert_eq!(entries.len(), 2, "only .bin payloads are listed");
        assert_eq!(entries[0].index, 1);
        assert_eq!(entries[0].name, "new");
        assert_eq!(entries[0].bytes, 2);
        assert_eq!(entries[1].name, "old");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_directory_lists_empty() {
        let entries = list_in(Path::new("/nonexistent/ak820"));
        assert!(entries.is_empty());
    }
}
