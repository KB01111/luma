use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use ignore::{WalkBuilder, WalkState};
use serde::{Deserialize, Serialize};

const CACHE_MAGIC: [u8; 8] = *b"LUMADX02";
const CACHE_VERSION: u32 = 2;
const PROGRESS_INTERVAL: usize = 2048;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    magic: [u8; 8],
    version: u32,
    built_at: u64,
    records: Vec<FileRecord>,
}

pub fn build_file_index(
    settings: &crate::config::IndexSettings,
    progress: impl Fn(usize) + Send + Sync,
) -> Result<Vec<FileRecord>, String> {
    let roots = default_roots(settings);
    let excluded: HashSet<String> = settings
        .excluded_names
        .iter()
        .map(|name| name.to_lowercase())
        .collect();
    let max_entries = settings.max_entries;
    let count = AtomicUsize::new(0);
    let records = Mutex::new(Vec::new());

    for (root_index, root) in roots.iter().enumerate() {
        let remaining = max_entries.saturating_sub(count.load(Ordering::Relaxed));
        if remaining == 0 {
            break;
        }
        let roots_left = roots.len().saturating_sub(root_index).max(1);
        let root_cap = remaining / roots_left;
        let root_count = AtomicUsize::new(0);

        let walker = WalkBuilder::new(root)
            .max_depth(Some(settings.max_depth))
            .threads(4)
            .hidden(false)
            .ignore(false)
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false)
            .parents(false)
            .follow_links(false)
            .build_parallel();

        walker.run(|| {
            let records = &records;
            let count = &count;
            let root_count = &root_count;
            let excluded = &excluded;
            let progress = &progress;
            Box::new(move |result| {
                let Ok(entry) = result else {
                    return WalkState::Continue;
                };
                if entry.depth() == 0 {
                    return WalkState::Continue;
                }
                let Some(file_type) = entry.file_type() else {
                    return WalkState::Continue;
                };
                if !file_type.is_file() && !file_type.is_dir() {
                    return WalkState::Continue;
                }

                let name = entry.file_name().to_string_lossy().into_owned();
                if name.is_empty() || is_noise_name(&name) || is_excluded_name(&name, excluded) {
                    return if file_type.is_dir() {
                        WalkState::Skip
                    } else {
                        WalkState::Continue
                    };
                }
                if file_type.is_dir() && name.starts_with('.') {
                    return WalkState::Skip;
                }

                let in_root = root_count.fetch_add(1, Ordering::Relaxed);
                if in_root >= root_cap {
                    return WalkState::Quit;
                }
                let accepted = count.fetch_add(1, Ordering::Relaxed);
                if accepted >= max_entries {
                    return WalkState::Quit;
                }

                let record = FileRecord {
                    path: entry.into_path(),
                    name,
                    is_dir: file_type.is_dir(),
                };
                {
                    let mut guard = records.lock().unwrap_or_else(|poison| poison.into_inner());
                    guard.push(record);
                }

                if (accepted + 1).is_multiple_of(PROGRESS_INTERVAL) {
                    progress(accepted + 1);
                }
                WalkState::Continue
            })
        });
    }

    let mut records = records
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner());
    records.sort_by_cached_key(|record| {
        let path = record.path.to_string_lossy();
        (path.to_lowercase(), path.into_owned())
    });
    records.dedup_by(|a, b| {
        a.path.to_string_lossy().to_lowercase() == b.path.to_string_lossy().to_lowercase()
    });

    Ok(records)
}

pub fn load_cached(_settings: &crate::config::IndexSettings) -> Option<(Vec<FileRecord>, u64)> {
    let bytes = fs::read(crate::config::cache_path()).ok()?;
    let cache: CacheFile = bincode::deserialize(&bytes).ok()?;
    if cache.magic != CACHE_MAGIC || cache.version != CACHE_VERSION {
        return None;
    }
    Some((cache.records, cache.built_at))
}

pub fn save_cache(records: &[FileRecord], built_at: u64) -> Result<(), String> {
    let cache = CacheFile {
        magic: CACHE_MAGIC,
        version: CACHE_VERSION,
        built_at,
        records: records.to_vec(),
    };
    let bytes = bincode::serialize(&cache).map_err(|error| error.to_string())?;
    crate::config::write_atomic(&crate::config::cache_path(), &bytes)
        .map_err(|error| error.to_string())
}

pub fn cache_is_fresh(built_at: u64, ttl_hours: u64) -> bool {
    if built_at == 0 || ttl_hours == 0 {
        return false;
    }
    let ttl_seconds = ttl_hours.saturating_mul(3_600);
    crate::config::unix_now().saturating_sub(built_at) <= ttl_seconds
}

pub fn default_roots(settings: &crate::config::IndexSettings) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    let home = dirs::home_dir();
    if let Some(home) = &home {
        add_root(&mut roots, &mut seen, home.clone());
    }

    let one_drive = std::env::var_os("OneDrive")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| {
            home.as_ref().and_then(|home| {
                let candidate = home.join("OneDrive");
                if candidate.is_dir() {
                    Some(candidate)
                } else {
                    None
                }
            })
        });

    if let Some(path) = one_drive {
        let inside_home = home.as_deref().is_some_and(|home| is_within(&path, home));
        if !inside_home {
            add_root(&mut roots, &mut seen, path);
        }
    }

    for drive in other_fixed_drives(home.as_deref()).into_iter().take(6) {
        add_root(&mut roots, &mut seen, drive);
    }

    for root in &settings.extra_roots {
        add_root(&mut roots, &mut seen, root.clone());
    }

    roots
}

#[cfg(windows)]
fn other_fixed_drives(home: Option<&Path>) -> Vec<PathBuf> {
    use std::os::windows::ffi::OsStrExt;

    use windows::{
        Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives},
        core::PCWSTR,
    };

    const DRIVE_FIXED: u32 = 3;

    let mask = unsafe { GetLogicalDrives() };
    let mut drives = Vec::new();
    for index in 0..26 {
        if mask & (1 << index) == 0 {
            continue;
        }
        let root = PathBuf::from(format!("{}:\\", (b'A' + index as u8) as char));
        let wide: Vec<u16> = root
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } != DRIVE_FIXED {
            continue;
        }
        if home.is_some_and(|home| is_within(home, &root)) {
            continue;
        }
        if root.is_dir() {
            drives.push(root);
        }
    }
    drives
}

#[cfg(not(windows))]
fn other_fixed_drives(_home: Option<&Path>) -> Vec<PathBuf> {
    Vec::new()
}

fn add_root(roots: &mut Vec<PathBuf>, seen: &mut HashSet<String>, path: PathBuf) {
    let path = normalized_path(&path);
    let key = path.to_string_lossy().to_lowercase();
    if !key.is_empty() && seen.insert(key) {
        roots.push(path);
    }
}

fn normalized_path(path: &Path) -> PathBuf {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    strip_verbatim_prefix(canonical)
}

#[cfg(windows)]
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

#[cfg(not(windows))]
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

fn is_within(path: &Path, parent: &Path) -> bool {
    let path = normalized_path(path).to_string_lossy().to_lowercase();
    let parent = normalized_path(parent).to_string_lossy().to_lowercase();
    path == parent
        || path
            .strip_prefix(parent.as_str())
            .is_some_and(|rest| rest.starts_with('\\') || rest.starts_with('/'))
}

fn is_noise_name(name: &str) -> bool {
    name.starts_with("~$")
        || name.eq_ignore_ascii_case("desktop.ini")
        || name.eq_ignore_ascii_case("thumbs.db")
        || name.eq_ignore_ascii_case(".ds_store")
}

fn is_excluded_name(name: &str, excluded: &HashSet<String>) -> bool {
    excluded.contains(&name.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_freshness_uses_ttl_hours() {
        let now = crate::config::unix_now();
        assert!(cache_is_fresh(now, 1));
        assert!(cache_is_fresh(now.saturating_sub(60), 24));
        assert!(!cache_is_fresh(now.saturating_sub(25 * 3_600), 24));
        assert!(!cache_is_fresh(now, 0));
        assert!(!cache_is_fresh(0, 24));
    }

    #[test]
    fn excluded_names_and_noise_are_case_insensitive() {
        let excluded = HashSet::from([
            "appdata".to_owned(),
            "node_modules".to_owned(),
            "start menu".to_owned(),
        ]);

        assert!(is_excluded_name("AppData", &excluded));
        assert!(is_excluded_name("NODE_MODULES", &excluded));
        assert!(is_excluded_name("Start Menu", &excluded));
        assert!(!is_excluded_name("Documents", &excluded));

        assert!(is_noise_name("desktop.ini"));
        assert!(is_noise_name("Thumbs.db"));
        assert!(is_noise_name(".DS_Store"));
        assert!(is_noise_name("~$document.docx"));
        assert!(!is_noise_name("desktop.ini.bak"));
        assert!(!is_noise_name("notes.txt"));
    }
}
