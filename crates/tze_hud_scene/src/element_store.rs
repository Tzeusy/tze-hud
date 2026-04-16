//! Persistent element identity store.
//!
//! Stores stable Scene IDs for zones, widgets, and tiles so IDs can survive
//! runtime restarts.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::types::{GeometryPolicy, SceneId};

/// Element category for persistent identity records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ElementType {
    Zone,
    Widget,
    Tile,
}

/// A persisted element identity entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElementStoreEntry {
    /// Kind of element this entry represents.
    pub element_type: ElementType,
    /// Stable namespace key (zone_name, widget instance_name, or tile namespace).
    pub namespace: String,
    /// Wall-clock creation time (milliseconds since Unix epoch).
    pub created_at: u64,
    /// Last publish/update timestamp (milliseconds since Unix epoch).
    pub last_published_at: u64,
    /// Optional user geometry override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry_override: Option<GeometryPolicy>,
}

/// Container for all persisted element identity entries.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ElementStore {
    /// Entries keyed by stable scene ID.
    #[serde(default)]
    pub entries: HashMap<SceneId, ElementStoreEntry>,
}

impl ElementStore {
    /// Parse a TOML-encoded element store.
    pub fn from_toml_str(input: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(input)
    }

    /// Serialize the store to TOML.
    pub fn to_toml_string(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Load the store from disk.
    ///
    /// Missing files are treated as first boot and return an empty store.
    pub fn load_or_default(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(content) => Self::from_toml_str(&content).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid element_store TOML: {err}"),
                )
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    /// Persist the store atomically using write-to-temp + replace.
    pub fn persist_to_path_atomic(&self, path: &Path) -> io::Result<()> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;

        let temp_path = unique_temp_path(path);
        let toml = self.to_toml_string().map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to serialize element_store TOML: {err}"),
            )
        })?;

        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)?;
        file.write_all(toml.as_bytes())?;
        file.sync_all()?;
        drop(file);

        if let Err(err) = replace_file_atomically(&temp_path, path) {
            let _ = fs::remove_file(&temp_path);
            return Err(err);
        }

        sync_parent_dir(parent)?;

        Ok(())
    }

    /// Find an entry by `(element_type, namespace)`.
    ///
    /// If duplicates exist, returns the oldest (then lexicographically smallest ID).
    pub fn find_id_by_type_namespace(
        &self,
        element_type: ElementType,
        namespace: &str,
    ) -> Option<SceneId> {
        let mut matches: Vec<(SceneId, &ElementStoreEntry)> = self
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                (entry.element_type == element_type && entry.namespace == namespace)
                    .then_some((*id, entry))
            })
            .collect();

        matches.sort_by_key(|(id, entry)| (entry.created_at, id.to_bytes_le()));
        matches.first().map(|(id, _)| *id)
    }
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("element_store.toml");
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    parent.join(format!(
        ".{stem}.tmp.{}.{}.{}",
        std::process::id(),
        now_ns,
        SceneId::new()
    ))
}

#[cfg(not(target_os = "windows"))]
fn replace_file_atomically(src: &Path, dst: &Path) -> io::Result<()> {
    fs::rename(src, dst)
}

#[cfg(target_os = "windows")]
fn replace_file_atomically(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    fn to_wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let src_w = to_wide(src);
    let dst_w = to_wide(dst);

    // SAFETY: pointers are valid NUL-terminated UTF-16 buffers for the duration
    // of the call and reference local immutable vectors.
    unsafe {
        MoveFileExW(
            PCWSTR(src_w.as_ptr()),
            PCWSTR(dst_w.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .ok()
        .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{err}")))?;
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn sync_parent_dir(path: &Path) -> io::Result<()> {
    OpenOptions::new().read(true).open(path)?.sync_all()
}

#[cfg(target_os = "windows")]
fn sync_parent_dir(_path: &Path) -> io::Result<()> {
    // Windows path replacement uses MoveFileExW with MOVEFILE_WRITE_THROUGH.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("tze_hud_scene_element_store_tests");
        let _ = fs::create_dir_all(&dir);
        dir.join(format!("{name}-{}.toml", SceneId::new()))
    }

    fn sample_store() -> ElementStore {
        let now = 1_710_000_000_000u64;
        let mut entries = HashMap::new();
        entries.insert(
            SceneId::new(),
            ElementStoreEntry {
                element_type: ElementType::Zone,
                namespace: "subtitle".to_string(),
                created_at: now,
                last_published_at: now,
                geometry_override: None,
            },
        );
        entries.insert(
            SceneId::new(),
            ElementStoreEntry {
                element_type: ElementType::Widget,
                namespace: "gauge-main".to_string(),
                created_at: now + 1,
                last_published_at: now + 1,
                geometry_override: Some(GeometryPolicy::Relative {
                    x_pct: 0.1,
                    y_pct: 0.2,
                    width_pct: 0.3,
                    height_pct: 0.4,
                }),
            },
        );
        ElementStore { entries }
    }

    #[test]
    fn toml_round_trip_preserves_entries() {
        let store = sample_store();
        let text = store.to_toml_string().expect("serialize");
        let restored = ElementStore::from_toml_str(&text).expect("deserialize");
        assert_eq!(restored, store);
    }

    #[test]
    fn missing_file_loads_empty_store() {
        let path = test_path("missing-load");
        let _ = fs::remove_file(&path);
        let store = ElementStore::load_or_default(&path).expect("load default");
        assert!(store.entries.is_empty());
    }

    #[test]
    fn persist_atomic_replaces_existing_file() {
        let path = test_path("atomic-replace");
        let _ = fs::remove_file(&path);

        let first = sample_store();
        first.persist_to_path_atomic(&path).expect("first persist");

        let mut second = ElementStore::default();
        second.entries.insert(
            SceneId::new(),
            ElementStoreEntry {
                element_type: ElementType::Tile,
                namespace: "agent.weather".to_string(),
                created_at: 7,
                last_published_at: 8,
                geometry_override: None,
            },
        );
        second
            .persist_to_path_atomic(&path)
            .expect("second persist");

        let restored = ElementStore::load_or_default(&path).expect("reload");
        assert_eq!(restored, second);

        let _ = fs::remove_file(&path);
    }
}
